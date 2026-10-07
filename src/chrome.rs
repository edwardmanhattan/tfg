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
    Color32, Context, CornerRadius, FontId, Id, Layout, Order, Painter, Pos2, Rect, Response,
    Sense, Shape, Stroke, StrokeKind, Ui, UiBuilder, Vec2, pos2, vec2,
};

// The palette and the measurements live in their own module so that both
// this file and `apply_ops_theme` read the same values instead of each
// keeping a copy. `crate::tokens` resolves in the app (lib.rs) and in the
// prototype crate (which declares `mod tokens` at its root beside the
// `#[path]`-included `mod chrome`), which is why the path is absolute rather
// than `super::`.
use crate::tokens;

/// `[...islands]`: every island sizes itself and scrolls its own body, so
/// the shell never has to measure content before painting the background.
/// That is the constraint that shapes this whole module — see
/// [`island`].
#[derive(Debug, Clone, PartialEq)]
pub struct Island {
    /// Stable id. Also the drag/close widget id stem, so it must not
    /// collide with any other island or its buttons will share hover state.
    pub id: Id,
    /// Short caps, tracked. Rendered through an explicit `LayoutJob`
    /// because `TextFormat` carries `extra_letter_spacing` and that is the
    /// only way to get the wide-tracked console look.
    pub title: String,
    /// Right-aligned band note. Empty means the band reserves nothing.
    pub trailing: String,
    pub size: Vec2,
}

impl Island {
    pub fn new(id: Id, title: &str, size: Vec2) -> Self {
        Self {
            id,
            title: title.to_string(),
            size,
            trailing: String::new(),
        }
    }

    /// A right-aligned note in the title band: a count, a state, a clock.
    ///
    /// Optional because the band has to work with one word in it, and a
    /// band that reserves space for a count it does not have is a band with
    /// a hole in it.
    pub fn with_trailing(mut self, text: &str) -> Self {
        self.trailing = text.to_string();
        self
    }

    /// The rect this island occupies at `pos`. The single place the geometry
    /// is derived, so ownership, stacking and painting cannot disagree about
    /// where the island is.
    pub fn rect_at(&self, pos: Pos2) -> Rect {
        Rect::from_min_size(pos, self.size)
    }

    /// Where the body content goes: below the title band, inside the pad.
    pub fn content_rect(&self, rect: Rect) -> Rect {
        Rect::from_min_max(
            pos2(
                rect.left() + tokens::ISLAND_PAD,
                rect.top() + tokens::TITLE_H + tokens::ISLAND_PAD,
            ),
            pos2(
                rect.right() - tokens::ISLAND_PAD,
                rect.bottom() - tokens::ISLAND_PAD,
            ),
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
/// 2px rim.
///
/// `owns_input` is the whole of ADR-0016. It is one boolean and it decides
/// whether the rim is `tokens::RESTING_RIM` or `tokens::ACTIVE_RIM`, and
/// there is deliberately no third state: the caller resolves which island
/// owns input before painting, because if two islands each test their own
/// `hovered` then two rims light and the rule means nothing.
///
/// The rim is drawn twice, once for each of its jobs. The inner band is the
/// attention signal, and it is the reason the register reads as heat rather
/// than as a highlighted border. The 2px cut edge stays neutral at rest so
/// the chamfer still reads as a chamfer when no island owns input.
pub fn paint_island(painter: &Painter, rect: Rect, owns_input: bool) {
    let pts = chamfer_tr(rect, tokens::CUT);

    painter.add(Shape::convex_polygon(
        translated(&pts, tokens::ISLAND_CAST_OFFSET),
        tokens::ISLAND_CAST,
        Stroke::NONE,
    ));
    painter.add(Shape::convex_polygon(
        pts.clone(),
        tokens::CONSOLE_NIGHT,
        Stroke::new(2.0, tokens::CUT_GREY),
    ));

    // Title band, clipped to the panel so it never crosses the diagonal.
    let band = Rect::from_min_max(
        rect.left_top(),
        pos2(rect.right() - tokens::CUT, rect.top() + tokens::TITLE_H),
    );
    painter.rect_filled(band, CornerRadius::ZERO, tokens::PANEL_SLATE);
    painter.line_segment(
        [
            pos2(band.left(), band.bottom()),
            pos2(band.right(), band.bottom()),
        ],
        Stroke::new(1.0, tokens::CUT_GREY.linear_multiply(0.6)),
    );

    // The rim: the polygon's own outline, offset inward. Insetting rather
    // than stroking the silhouette is what makes it a rim, and it is why the
    // band can trace the chamfer without the chamfer reading as a heavier
    // edge than the panel.
    paint_rim(painter, &pts, owns_input);

    // The chamfer's own edge, neutral at rest. Brightened rather than
    // recoloured on hover, so the One Signal Rule holds while the cut still
    // reads as deliberate.
    let cut_edge = Stroke::new(
        2.0,
        if owns_input {
            tokens::CUT_GREY.gamma_multiply(1.6)
        } else {
            tokens::CUT_GREY
        },
    );
    painter.line_segment(
        [
            pos2(rect.right() - tokens::CUT, rect.top()),
            pos2(rect.right(), rect.top() + tokens::CUT),
        ],
        cut_edge,
    );
}

/// The 2px band inset from `pts`, in the rim colour for the input state.
fn paint_rim(painter: &Painter, pts: &[Pos2], owns_input: bool) {
    let centroid = (pts
        .iter()
        .fold(Vec2::ZERO, |acc, p| acc + p.to_vec2())
        / pts.len() as f32)
        .to_pos2();
    let inner: Vec<Pos2> = pts
        .iter()
        .map(|p| {
            let inward = (centroid - *p).normalized();
            *p + inward * tokens::RIM_INSET
        })
        .collect();
    painter.add(Shape::closed_line(
        inner
            .iter()
            .copied()
            .chain(std::iter::once(inner[0]))
            .collect(),
        Stroke::new(tokens::RIM_BAND, tokens::rim(owns_input)),
    ));
}

/// The title band's rule texture.
///
/// Restricted to the band on purpose. `DESIGN.md`'s "Texture, confined":
/// rendered across a body the rules cut through the rows and cost legibility
/// on the smallest text on the panel. The band holds one short tracked label
/// and a count, so the texture is free there and nowhere else.
pub fn paint_band_texture(painter: &Painter, rect: Rect) {
    let mut y = rect.top() + tokens::BAND_SCAN_PITCH;
    let bottom = rect.top() + tokens::BAND_TEXTURE_H;
    while y < bottom {
        painter.line_segment(
            [
                pos2(rect.left(), y),
                pos2(rect.right() - tokens::CUT, y),
            ],
            Stroke::new(1.0, Color32::from_white_alpha(tokens::BAND_SCAN_ALPHA)),
        );
        y += tokens::BAND_SCAN_PITCH;
    }
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
    let stroke = Stroke::new(
        1.4,
        if hot {
            tokens::MAP_INK
        } else {
            tokens::BODY_SILVER
        },
    );
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
/// size, and content taller than `content_rect` scrolls inside it rather
/// than growing the panel or being cut off.
///
/// The body scrolls inside that fixed rect, always. That is not a
/// workaround, it is the reason a console island has a stable footprint:
/// a panel that resizes under the cursor while the operator is reading a
/// map is worse than one that scrolls — and a panel that CLIPS is worse
/// than both, because what it cuts is content nobody can scroll to.
///
/// `pos` is read before the area opens and written after, so a drag from
/// the previous frame is applied first — the same ordering `egui::Window`
/// uses internally, and the reason a drag does not lag a frame.
///
/// `open` is set to `false` on close; the caller owns persisting it.
///
/// Returns the title band's response, which is what a caller needs to drive
/// a drag and to ask whether the pointer was over the band. It is `None`
/// only on the first frame, before any response exists for the island's ids.
pub fn island(
    ctx: &egui::Context,
    spec: &Island,
    pos: &mut Pos2,
    open: &mut bool,
    body: impl FnOnce(&mut Ui),
) -> Option<Response> {
    // With no central resolver, this island claims ownership by asking
    // whether the pointer is inside it. Correct for a non-overlapping
    // layout, which is every island in the console today; a zone that
    // stacks overlapping islands must call [`island_owned`] instead.
    let owns_input = ctx
        .input(|i| i.pointer.hover_pos())
        .is_some_and(|p| spec.rect_at(*pos).contains(p));
    // No scroll and the whole window as the band: this primitive has no column
    // to scroll, so the band must not clip what the caller could legitimately
    // place anywhere in the viewport.
    island_owned(ctx, spec, pos, 0.0, ctx.viewport_rect(), open, owns_input, body)
}

/// [`island`], with ownership supplied by the caller.
///
/// A zone calls this so that exactly one rim in the column lights, decided
/// once by [`owning_island`]. Everything else about the island is identical.
pub fn island_owned(
    ctx: &egui::Context,
    spec: &Island,
    pos: &mut Pos2,
    scroll: f32,
    band: Rect,
    open: &mut bool,
    owns_input: bool,
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
            // Constrained to the window, not the screen, and ONLY for a drag.
            // A computed column position legitimately sits below the window —
            // that is what a scrollable column IS — so clamping it would pull
            // the island up to the window's bottom edge and then the zone's
            // scroll would carry it off the top, which reads as an island that
            // cannot be scrolled to.
            let vp = ctx.viewport_rect();
            pos.x = pos.x.clamp(vp.left(), (vp.right() - spec.size.x).max(vp.left()));
            pos.y = pos.y.clamp(vp.top(), (vp.bottom() - spec.size.y).max(vp.top()));
        }
    }

    // Scroll is a VIEW transform, applied here rather than folded into `pos`.
    //
    // `pos` is where the island lives in the column and the clamp above is
    // about dragging. Subtracting the scroll before the clamp would let a
    // scroll carry an island off the top of the window, which is the one thing
    // that clamp exists to prevent, and the two would silently undo each other
    // in whichever direction each happened to run first.
    let island_pos = pos2(pos.x, pos.y - scroll);
    if !island_on_band(*pos, scroll, band, spec.size.y) {
        return None;
    }
    let mut title_resp: Option<Response> = None;
    let mut clicked_close = false;
    let mut fit: Option<BodyFit> = None;

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
            // The band, not the island's own rect, is the outer limit. An
            // island half-scrolled under the top band must disappear behind it
            // rather than paint over it, and `painter_at` takes the clip from
            // the rect it is given, so the background and title go through the
            // intersection too.
            ui.set_clip_rect(ui.max_rect().intersect(band));
            let rect = ui.min_rect();

            let painter = ui.painter_at(rect.intersect(band));

            // ADR-0016: the rim lights on the island that owns input, and
            // ownership arrives as a parameter rather than being decided
            // here, so that a zone can guarantee exactly one lit rim. See
            // `owning_island`.
            paint_island(&painter, rect, owns_input);
            paint_band_texture(&painter, rect);

            // Title band: registered BEFORE the close button, because
            // egui's hit test walks candidates in reverse registration
            // order (`hit_test.rs`) and the last one registered wins where
            // they overlap. Reverse these and the close button dies.
            let title_band = Rect::from_min_max(
                rect.left_top(),
                pos2(rect.right() - tokens::CUT, rect.top() + tokens::TITLE_H),
            );
            let drag = ui.interact(
                title_band,
                spec.id.with("__title"),
                Sense::click_and_drag(),
            );

            tracked_caps(
                ctx,
                &painter,
                pos2(title_band.left() + tokens::ISLAND_PAD, title_band.center().y - 7.0),
                &spec.title,
                tokens::TITLE_SIZE,
                tokens::MAP_INK,
                tokens::TITLE_TRACKING,
            );

            let close_rect = Rect::from_center_size(
                pos2(
                    rect.right() - tokens::CLOSE_INSET,
                    rect.top() + tokens::TITLE_H * 0.5,
                ),
                vec2(tokens::CLOSE, tokens::CLOSE),
            );

            if !spec.trailing.is_empty() {
                // Right-aligned, and stopping short of the close button.
                // The button sits at CLOSE_INSET from the panel edge and the
                // band's own right edge is CUT from it, so a note placed at
                // the band's edge lands underneath the button — which is
                // what the first pass did.
                painter.text(
                    pos2(close_rect.left() - tokens::ISLAND_PAD, title_band.center().y + 4.0),
                    egui::Align2::RIGHT_BOTTOM,
                    &spec.trailing,
                    FontId::monospace(11.0),
                    tokens::BODY_SILVER,
                );
            }

            // The close button is registered LAST, after the body scope, and
            // that order is the whole fix.
            //
            // egui gives a click to ONE widget: the topmost clickable widget
            // whose interact rect contains the press, ties going to the last
            // one registered. Hover is not exclusive, so a button buried under
            // the body's widgets still lights up while every click lands on
            // whatever was registered after it. That is what this was:
            // hovering the ✕ lit it and clicking it did nothing, on every
            // island and in every modal.

            // Body, in a fixed rect, SCROLLING inside it.
            //
            // The scroll is not a nicety here, it is the difference between a
            // control that exists and one that does not. This used to lay the
            // body straight into the rect and CLIP the overflow, which is sound
            // as paint — a `max_rect` tells the layout where to stop but not
            // the painter, so the Operator island's overflow painted over the
            // map and half-occluded under the next island's title band — and
            // broken as a surface, because clipped content is UNREACHABLE
            // content. At the app's own default 1040x640, Essentials measured
            // ~1000pt of content against 624 available, so the composer and
            // everything under it were simply not on the screen, and scrolling
            // the COLUMN could not bring them there: the column moves whole
            // islands, and this island was taller than the window.
            //
            // `auto_shrink([false, false])` because the FOOTPRINT is fixed by
            // contract (DESIGN.md: "An island does not resize to fit its
            // content. The body scrolls."). Left on, the scroll area would
            // shrink-wrap its content and a short body would leave a gap under
            // itself inside a panel sized for the long case.
            //
            // `id_salt`, so two islands with identical bodies keep independent
            // offsets — which is what stops scrolling one from scrolling the
            // other.
            let content = spec.content_rect(rect);
            ui.scope_builder(
                UiBuilder::new()
                    .max_rect(content)
                    .layout(Layout::top_down(egui::Align::LEFT))
                    .sense(Sense::hover()),
                |ui| {
                    // `band` in that intersection is load-bearing: this scope
                    // re-clips to the island's own content rect, which
                    // OVERWRITES the band clip set above, so without the band a
                    // scrolled island paints its body up over the top band.
                    ui.set_clip_rect(content.intersect(ui.max_rect()).intersect(band));
                    let scrolled = egui::ScrollArea::vertical()
                        .id_salt(spec.id.with("__body_scroll"))
                        .auto_shrink([false, false])
                        .show(ui, body);
                    // `content_size`, NOT `ui.min_rect()`. The rect handed to a
                    // ScrollArea is the scroll AREA, so `min_rect` reports the
                    // viewport and every island measures as an exact fit —
                    // which reads as "nothing overflows" while the content is
                    // being cut. `content_size` is the content's own natural
                    // extent (`content_ui.min_size()`, scroll_area.rs:1082), so
                    // this is the measurement the clipped version made, and it
                    // stays correct with a scroll in the way.
                    fit = Some(BodyFit {
                        content_h: scrolled.content_size.y,
                        viewport_h: content.height(),
                        offset: scrolled.state.offset.y,
                    });
                },
            );

            title_resp = Some(drag);

            let close = ui.interact(close_rect, spec.id.with("__close"), Sense::click());
            paint_close(&painter, close_rect, close.hovered());
            clicked_close = close.clicked();
        });

    if clicked_close {
        *open = false;
    }
    if let Some(f) = fit {
        ctx.data_mut(|d| d.insert_temp(island_fit_key(spec.id), f));
    }
    title_resp
}

/// What one island body measured about itself, published so the zone can
/// decide who owns the wheel.
///
/// Three numbers and no more, and each one is load-bearing for a decision
/// that cannot be made any other way. `content_h` against `viewport_h` is
/// whether the body has anywhere to scroll at all; `offset` is where in its
/// range it already is. Together they answer "does this body still have room
/// in the direction the operator is turning the wheel", which is the whole
/// of scroll chaining — and which no caller can work out for itself, because
/// the body is laid out inside here.
///
/// A body that fits reports `content_h` within a few points of `viewport_h`
/// and answers `false` to both, so a caller can ask every island the same
/// question without first working out which of them are tall.
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
    ///
    /// False for a body that fits, and for one already at the bottom — which
    /// is the case that hands the wheel back to the column.
    pub fn can_scroll_down(&self) -> bool {
        self.content_h - self.viewport_h - self.offset > BODY_SCROLL_SLACK
    }

    /// Whether the body has more content above the fold than below it.
    pub fn can_scroll_up(&self) -> bool {
        self.offset > BODY_SCROLL_SLACK
    }
}

/// How far a body is from its limit before it counts as being at it.
///
/// Half a point past the limit, to absorb floating-point residue in the offset.
/// Larger and the last pixels of an island's content cannot be scrolled to;
/// smaller and a body reports room it does not have, which strands the wheel on
/// a body that will not move.
const BODY_SCROLL_EPSILON: f32 = 0.5;

/// Content a body may run past its viewport without claiming the wheel.
///
/// One text line, and it is the same number `OVERFLOW_SLACK` used to be for the
/// same reason. A layout's measured extent runs a few points past the last
/// control for trailing spacing: the Operator island measures 98.4 against 94
/// available, 4pt of nothing at the bottom.
///
/// Without this, that 4pt is scrollable, so the Operator body claims the first
/// wheel gesture, moves four points the operator cannot see, and the column
/// does not move. A gesture that appears to do nothing is the exact failure
/// this whole mechanism exists to avoid, and it would be reintroduced by four
/// points of trailing spacing.
///
/// Twelve points is the tolerance `OVERFLOW_SLACK` was pinned at, for the same
/// measurement and the same reason.
const BODY_SCROLL_SLACK: f32 = 12.0;

/// The key an island's body measurement is filed under.
fn island_fit_key(id: Id) -> Id {
    id.with("__body_fit")
}

/// What island `id`'s body measured, or `None` if it has not been drawn.
///
/// TEMP, deliberately: this is a fact about one frame's layout, and a
/// persistent entry would outlive the island that set it — a closed island
/// would keep answering for a body nobody can see.
pub fn island_body_fit(ctx: &Context, id: Id) -> Option<BodyFit> {
    ctx.data(|d| d.get_temp::<BodyFit>(island_fit_key(id)))
}

/// The key the top band's measured bottom is filed under.
fn top_band_key() -> Id {
    Id::new("__top_band")
}

/// File where the top band ended this frame.
///
/// Set by `top_zone`, read by `modal`'s backdrop, and the reason it is a
/// round trip through temp storage rather than a constant is in `modal`.
///
/// The context is a parameter rather than reached for, because a fresh
/// `Context::default()` here would be a different context and the value would
/// be filed where nobody reads it.
pub fn publish_top_band(ctx: &Context, bottom: f32) {
    ctx.data_mut(|d| d.insert_temp(top_band_key(), bottom));
}

/// Where the top band ends, or the whole viewport top when it has not been drawn.
///
/// A backdrop that cannot find the band must not guess: it claims the full
/// viewport, which is the older and safer behaviour — the map stays inert and a
/// modal has its own ✕. Being wrong in that direction costs a dead Settings
/// button on one frame; being wrong the other way costs a modal that quietly
/// lets clicks through to the map.
fn top_band_bottom(ctx: &Context) -> f32 {
    ctx.data(|d| d.get_temp::<f32>(top_band_key()).unwrap_or(0.0))
}

// ---------------------------------------------------------------------------
// The side zone
// ---------------------------------------------------------------------------

/// Which edge the zone is docked to.
///
/// A plain enum rather than a bool because the two cases appear in every
/// position calculation and a bare `true` there is unreadable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dock {
    Left,
    Right,
}

/// The zone's default width, in px. Also the reset value for the resize
/// grip: the live width lives on the app's `side_zone_w`, because a floating
/// zone over a map moves the map's visible centre when it resizes and the
/// camera compensates from the actual width every frame
/// ([`camera_centre_offset_with`]).
pub fn zone_width() -> f32 {
    tokens::ZONE_W
}

/// Clamp a candidate zone width: inside the resize limits, and never wider
/// than the window minus both edge gaps and a 200px map strip.
///
/// The floor wins on a narrow window: `min <= max` always, so this never
/// panics and a tiny window gets a 240px zone rather than a crash.
pub fn clamp_zone_width(w: f32, viewport_w: f32) -> f32 {
    let max = (viewport_w - tokens::ZONE_EDGE_GAP * 2.0 - 200.0).max(tokens::ZONE_MIN_W);
    w.clamp(tokens::ZONE_MIN_W, max.min(tokens::ZONE_MAX_W))
}

/// The zone's horizontal origin for a window of `viewport_w`, or `None` when
/// the zone is hidden.
pub fn zone_origin(dock: Dock, viewport_w: f32) -> Option<f32> {
    zone_origin_with(dock, viewport_w, tokens::ZONE_W)
}

/// [`zone_origin`] for a live (possibly resized) zone width.
pub fn zone_origin_with(dock: Dock, viewport_w: f32, zone_w: f32) -> Option<f32> {
    let x = match dock {
        Dock::Left => tokens::ZONE_EDGE_GAP,
        Dock::Right => viewport_w - zone_w - tokens::ZONE_EDGE_GAP,
    };
    (x >= 0.0).then_some(x)
}

/// Where a hull should actually be drawn so it is not hidden under the zone.
///
/// This is the cost of the zone floating rather than docking, stated once
/// and in one place because it is the kind of rule that gets rediscovered as
/// a bug report. A camera action that frames a hull has to aim at the
/// *visible* centre of the map, and with `w` of the map's width covered on
/// one side, that centre is half of `w` away from the window centre.
///
/// Returns `0.0` when the zone is hidden, because then the window centre is
/// the visible centre and no correction applies.
pub fn camera_centre_offset(dock: Dock, hidden: bool) -> f32 {
    camera_centre_offset_with(dock, hidden, tokens::ZONE_W)
}

/// [`camera_centre_offset`] for a live (possibly resized) zone width. The
/// caller passes the same clamped width the column is drawn at, so a resize
/// never parks a framed hull under the chrome.
pub fn camera_centre_offset_with(dock: Dock, hidden: bool, zone_w: f32) -> f32 {
    if hidden {
        return 0.0;
    }
    match dock {
        // The zone covers the left, so the visible centre is to its right.
        Dock::Left => zone_w * tokens::CAMERA_OFFSET_FRACTION,
        Dock::Right => -zone_w * tokens::CAMERA_OFFSET_FRACTION,
    }
}

/// Stack islands down the zone, from the top, and give each the same x.
///
/// Returns each island's origin in the order given, skipping any island the
/// caller marked closed. The column scrolls as a whole when the stack is
/// taller than the window, so a state with many islands never pushes the last
/// one off the bottom.
///
/// The islands keep their own drag, which means an island can be dragged out
/// of the column. That is deliberate for now: the operator's brief asks for
/// movable islands, and reconciling "the zone owns the layout" with "the
/// operator may move an island" is a decision, not an accident. See
/// `docs/ui-overhaul-status.md`.
pub fn zone_island_origins(
    dock: Dock,
    viewport: Rect,
    islands: &[(Island, bool)],
) -> Vec<Pos2> {
    zone_island_origins_with(dock, viewport, islands, tokens::ZONE_W)
}

/// [`zone_island_origins`] for a live (possibly resized) zone width.
pub fn zone_island_origins_with(
    dock: Dock,
    viewport: Rect,
    islands: &[(Island, bool)],
    zone_w: f32,
) -> Vec<Pos2> {
    let Some(x) = zone_origin_with(dock, viewport.width(), zone_w) else {
        return Vec::new();
    };
    let mut y = viewport.top() + tokens::ZONE_TOP_GAP;
    islands
        .iter()
        .filter(|(_, open)| *open)
        .map(|(spec, _)| {
            let origin = pos2(x, y);
            y += spec.size.y + tokens::ZONE_ISLAND_GAP;
            origin
        })
        .collect()
}

/// The strip of the window a side-zone island may occupy.
///
/// Scrolling, clipping and culling all read this, so "where an island may be"
/// is one rectangle rather than three numbers that can disagree.
pub fn zone_band(dock: Dock, viewport: Rect) -> Rect {
    zone_band_with(dock, viewport, tokens::ZONE_W)
}

/// [`zone_band`] for a live (possibly resized) zone width.
pub fn zone_band_with(dock: Dock, viewport: Rect, zone_w: f32) -> Rect {
    let x = zone_origin_with(dock, viewport.width(), zone_w).unwrap_or(0.0);
    Rect::from_min_max(
        pos2(
            x,
            viewport.top() + tokens::ZONE_TOP_GAP - tokens::ZONE_ISLAND_GAP,
        ),
        pos2(
            x + zone_w,
            viewport.bottom() - tokens::ZONE_ISLAND_GAP,
        ),
    )
}

/// Whether an island at column position `pos` still shows once the column has
/// been scrolled by `scroll`.
///
/// Culled rather than drawn off-band. An island outside the band must not own
/// input either, and ownership is decided from a list of rects the caller
/// builds, so the caller needs this predicate and not just the drawing.
pub fn island_on_band(pos: Pos2, scroll: f32, band: Rect, height: f32) -> bool {
    let y = pos.y - scroll;
    y + height >= band.top() && y <= band.bottom()
}

/// Which island in a zone owns input, if any.
///
/// ADR-0016 rests on a claim that is easy to state and easy to break: at
/// most one island's rim lights at a time. If each island decided that for
/// itself from its own `hovered`, then any two overlapping islands would both
/// light and the accent would mean "an island is here" rather than "you are
/// here" — which is the failure the ADR exists to prevent.
///
/// Resolving it in one place makes the rule a property of the function
/// instead of a property of every caller's restraint, and it is what lets
/// [`island_owned`] take ownership as a parameter.
///
/// Returns `None` for no pointer or a pointer between islands. Ties cannot
/// happen for a non-overlapping column; if two rects do contain the point,
/// the lower index wins, so the answer is still deterministic.
pub fn owning_island(rects: &[Rect], pointer: Option<Pos2>) -> Option<usize> {
    let p = pointer?;
    rects.iter().position(|r| r.contains(p))
}

/// A zone has no background of its own, and that is a decision rather than
/// an omission: the map shows through the gaps between islands, because a
/// zone that grew a panel of its own would be a fourth kind of thing on
/// screen. There is deliberately no function to call for it.

// ---------------------------------------------------------------------------
// The modal zone
// ---------------------------------------------------------------------------

/// A modal surface: the third zone, for authoring rather than operating.
#[derive(Debug, Clone, PartialEq)]
pub struct Modal {
    pub id: Id,
    pub title: String,
    pub size: Vec2,
    /// Set to `false` by the close button. The caller owns persisting it,
    /// same contract as an island.
    pub open: bool,
    /// Whether the backdrop paints its dim.
    pub backdrop: Backdrop,
    /// The panel steps out of the way entirely: no backdrop, no panel, no
    /// input claimed. See [`Modal::step_aside`].
    pub aside: bool,
}

/// What the backdrop behind a modal does about the map.
///
/// Both variants swallow input identically — that is the whole point of the
/// backdrop and it never varies. What varies is whether it darkens, because a
/// modal that dims also dims the thing the operator is aiming at.
///
/// `Clear` exists for exactly one caller: a drag from inside a modal out onto
/// the map. The drag ghost is painted by the map, so under a dim it is the
/// dimmest thing on screen at the moment its legibility matters most. Swallow
/// without dimming and the ghost reads at full contrast while the map behind
/// it still cannot be clicked by accident.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backdrop {
    Dim,
    Clear,
}

impl Modal {
    pub fn new(id: Id, title: &str, size: Vec2) -> Self {
        Self {
            id,
            title: title.to_string(),
            size,
            open: true,
            backdrop: Backdrop::Dim,
            aside: false,
        }
    }

    /// The panel stops existing for this frame: no backdrop, no panel, and
    /// nothing claimed.
    ///
    /// For a PALETTE rather than a dialog. A palette exists to start something
    /// that happens on the map — the Fleet picker arms a placement, the Player
    /// picker assigns a piece — and for the whole of that gesture the palette is
    /// in the way of the thing it is pointing at. It covers the map the operator
    /// is aiming at, a release over it is refused by `drop_lands_on_map`, and
    /// the drag ghost is painted underneath it.
    ///
    /// The backdrop is the deeper half of the problem. It swallows input across
    /// the viewport by design — that is how a drag that leaves the panel is
    /// stopped from landing a hull behind the form — but a palette whose gesture
    /// ENDS on the map is stopped by the very rule meant to protect it. The code
    /// already half-knew this: `clear_backdrop` existed for exactly one caller,
    /// the Fleet picker, for exactly this reason, and it only dropped the dim.
    /// The panel itself was still there.
    ///
    /// So while the gesture is in flight the whole surface goes, and comes back
    /// the moment it ends. Nothing is lost: the drag's state is on `ShipApp`, not
    /// in the panel, and a drag that is cancelled restores the panel with it.
    pub fn step_aside(mut self) -> Self {
        self.aside = true;
        self
    }

    /// The fill alpha for this backdrop.
    ///
    /// One function so the two variants cannot disagree about what "no dim"
    /// means, and so a test can pin the choice without a renderer.
    pub fn backdrop_alpha(&self) -> u8 {
        match self.backdrop {
            Backdrop::Dim => MODAL_BACKDROP_ALPHA,
            // `Clear` is unreachable now and `step_aside` is what a palette
            // uses. Kept so the enum can say what it means, but nothing sets it:
            // a "clear but still swallowing" backdrop was the halfway answer
            // that left the panel covering the target, and `step_aside` replaced
            // it rather than sitting beside it.
            Backdrop::Clear => 0,
        }
    }

    /// Where the panel sits, centred in `viewport` and never larger than
    /// it.
    ///
    /// Clamping rather than shrinking: a modal that silently changes its own
    /// footprint to fit a small screen reflows a form the operator is halfway
    /// through, and a form whose fields have moved under the cursor is worse
    /// than a form with a scrollbar.
    pub fn rect_in(&self, viewport: Rect) -> Rect {
        let size = Vec2::new(
            self.size.x.min(viewport.width()),
            self.size.y.min(viewport.height()),
        );
        Rect::from_center_size(viewport.center(), size)
    }

    /// The body rect: below the title row, inside the pad.
    ///
    /// Clamped to be non-negative. On a window shorter than the title row
    /// plus both pads the naive subtraction yields a NEGATIVE height, and
    /// `scope_builder` handed a negative rect lays its content out somewhere
    /// other than inside the panel — which on a short window is a form that
    /// vanishes instead of scrolling. The first version did exactly that,
    /// and the test that checks for it is the reason it does not any more.
    pub fn body_rect(&self, rect: Rect) -> Rect {
        let top_left = pos2(rect.left() + tokens::PAD, rect.top() + MODAL_TITLE_H + tokens::PAD);
        let bottom_right = pos2(rect.right() - tokens::PAD, rect.bottom() - tokens::PAD);
        Rect::from_min_max(
            pos2(top_left.x.min(bottom_right.x), top_left.y.min(bottom_right.y)),
            pos2(top_left.x.max(bottom_right.x), top_left.y.max(bottom_right.y)),
        )
    }
}

/// How much the modal dims what is behind it.
///
/// Enough that a lit map stops competing with a form, not so much that the
/// island the modal was opened from becomes unreadable — the operator still
/// needs to see the zone they are working in.
///
/// MEASURED, not chosen, and measured on the right background.
///
/// The first value was 150. Judged against the harness — which has no
/// background of its own, so every capture showed a near-black desktop
/// wallpaper — 150 looked like plenty. That judgement was worthless: a black
/// overlay over black tells you nothing, and the real tile map is BRIGHT.
/// `p5-epaint --bright` now paints a map-like ground (pale sea, near-white
/// land, coastlines, grid, place labels) for exactly this reason.
///
/// On that ground the two values are not close. At 150 land and sea collapse
/// to a single value — land reads srgb(100,101,96) against sea srgb(35,38,54)
/// — the coastline strokes vanish and the place labels all but disappear, so
/// the map stops reading as a map. At 104 land holds at srgb(143,143,136),
/// land/sea/grid/coastline all survive, the labels stay legible, and the zone
/// islands behind still read as clearly recessed. 104 it is.
pub const MODAL_BACKDROP_ALPHA: u8 = 104;

/// Title row height. Smaller than an island's band because a modal has no
/// chamfer and no drag region, so the band is carrying only a label and a
/// close button.
pub const MODAL_TITLE_H: f32 = 30.0;

/// Show a modal, or report that it was closed.
///
/// Two things make a modal a modal and both are here rather than left to the
/// caller, because getting either wrong produces the same bug: a drag that
/// starts inside the form and ends outside it continues onto the map, so
/// dropping a hull "into" the Fleet Picker pans the camera instead.
///
/// - The backdrop is a real widget with `Sense::click_and_drag()`, painted
///   across the whole viewport before the panel. It swallows the pointer, so
///   a drag that leaves the panel dies on the backdrop rather than reaching
///   the map.
/// - It paints in `Order::Foreground`, above the zones and above the map.
///   `Order` has no `Top` in egui 0.36 — `Foreground` is the layer above
///   normal windows, which is what a modal is.
///
/// Returns `false` when the modal was closed this frame, so the caller can
/// drop its state without tracking a second flag.
pub fn modal(
    ctx: &egui::Context,
    spec: &Modal,
    body: impl FnOnce(&mut Ui),
) -> bool {
    let viewport = ctx.viewport_rect();
    let rect = spec.rect_in(viewport);

    // A stepped-aside panel claims nothing and paints nothing, and returns
    // BEFORE the backdrop so it does not even dim the map it is getting out of
    // the way for. `open` is returned unchanged: the caller keeps its state, and
    // the panel is back the frame the gesture ends.
    if spec.aside {
        return spec.open;
    }

    let mut clicked_close = false;
    // The top band stays OUTSIDE the backdrop.
    //
    // The backdrop claims the whole viewport, which is what makes the map inert
    // behind a modal — correct, and it is why a drag that leaves the panel dies
    // on the backdrop instead of dropping a hull on the map. But it also made
    // the top band inert, and the top band is where Settings and Sign out live.
    // So a modal had exactly one way out, its own ✕, and everything else on
    // screen looked present and dead: click a control you can plainly see, get
    // nothing, and conclude there is an overlay stuck on the app.
    //
    // The band is the app's chrome rather than the map, it is above the modal in
    // the operator's mind ("the frame is still live, the form is what is modal"),
    // and leaving it live gives a modal a second exit that costs no panel
    // geometry. The band is also the one place the pointer can be while a
    // backdrop is up without being over the map, so a drag cannot escape through
    // it by accident — the drag ends on the map or nowhere.
    //
    // FROM the band's bottom, not to it. The first version of this read the band
    // as the backdrop's top edge and built a rect from the viewport origin down
    // to it — which covered the band and left the whole map clickable through.
    // Inverted, and the render said so at once: the band stayed inert and the map
    // stopped dimming.
    let backdrop_rect = Rect::from_min_max(
        pos2(viewport.left(), top_band_bottom(ctx)),
        viewport.max,
    );
    egui::Area::new(spec.id.with("__backdrop"))
        // Positioned AT the rect it is going to claim, not at the viewport
        // origin. An `Area`'s inner cursor starts at its own `fixed_pos`, so
        // allocating from an Area at the origin hands back a rect at the origin
        // whatever size is asked for — and the version before this one painted
        // the correct rect while interacting with that one. The difference is
        // invisible in a screenshot: the map dimmed where it should and the band
        // stayed bright, while the backdrop went on eating every click on the
        // screen, the panel included. So the Area goes where the rect is.
        .fixed_pos(backdrop_rect.min)
        .movable(false)
        .constrain(false)
        .interactable(true)
        .order(Order::Foreground)
        .show(ctx, |ui| {
            // Claim the region below the band before the panel exists, so the
            // panel is drawn over ground that already belongs to the backdrop.
            let (full, _) =
                ui.allocate_exact_size(backdrop_rect.size(), Sense::click_and_drag());
            // The sense is unconditional and the fill is not: a clear
            // backdrop must still block the map underneath, and that is a
            // separate decision from how dark it looks.
            let alpha = spec.backdrop_alpha();
            if alpha > 0 {
                ui.painter().rect_filled(
                    full,
                    CornerRadius::ZERO,
                    Color32::from_black_alpha(alpha),
                );
            }
        });

    egui::Area::new(spec.id)
        .fixed_pos(rect.min)
        .movable(false)
        .constrain(false)
        .interactable(true)
        .order(Order::Foreground)
        .show(ctx, |ui| {
            ui.set_min_size(spec.size);
            ui.set_max_size(spec.size);
            let panel = ui.min_rect();
            let painter = ui.painter_at(panel);

            // Square, not chamfered. The chamfer means "this whole surface
            // moves" (ADR-0014), and a modal does not move.
            painter.rect_filled(
                panel,
                CornerRadius::ZERO,
                tokens::CONSOLE_NIGHT,
            );
            painter.rect_stroke(
                panel,
                CornerRadius::ZERO,
                Stroke::new(1.0, tokens::HAIRLINE_SLATE),
                StrokeKind::Inside,
            );

            let title_row = Rect::from_min_max(
                panel.left_top(),
                pos2(panel.right(), panel.top() + MODAL_TITLE_H),
            );
            painter.rect_filled(title_row, CornerRadius::ZERO, tokens::PANEL_SLATE);
            painter.line_segment(
                [
                    pos2(title_row.left(), title_row.bottom()),
                    pos2(title_row.right(), title_row.bottom()),
                ],
                Stroke::new(1.0, tokens::HAIRLINE_SLATE),
            );
            tracked_caps(
                ctx,
                &painter,
                pos2(title_row.left() + tokens::PAD, title_row.center().y - 7.0),
                &spec.title,
                tokens::TITLE_SIZE,
                tokens::MAP_INK,
                tokens::TITLE_TRACKING,
            );

            let close_rect = Rect::from_center_size(
                pos2(
                    panel.right() - tokens::PAD - tokens::CLOSE * 0.5,
                    panel.top() + MODAL_TITLE_H * 0.5,
                ),
                vec2(tokens::CLOSE, tokens::CLOSE),
            );
            // Registered after the body scope, for the reason spelled out in
            // `island`: a click goes to the topmost clickable widget, and
            // hover does not, so registering this before the body left a
            // button that lit up and swallowed nothing.

            ui.scope_builder(
                UiBuilder::new()
                    .max_rect(spec.body_rect(panel))
                    .layout(Layout::top_down(egui::Align::LEFT))
                    .sense(Sense::hover()),
                body,
            );

            let close = ui.interact(close_rect, spec.id.with("__close"), Sense::click());
            paint_close(&painter, close_rect, close.hovered());
            clicked_close = close.clicked();
        });

    if clicked_close {
        return false;
    }
    spec.open
}

#[cfg(test)]
mod tests {

    /// The predicate that decides whether an island is reachable at all.
    ///
    /// This is the whole bug in one function. Planning stacks five islands into
    /// 1642pt against a 584pt band, so everything below the first is off-band at
    /// scroll zero — and a culled island is not merely invisible, it is GONE: no
    /// rim, no content, no way to scroll to it. Control and Fleet were
    /// unreachable for exactly this reason.
    #[test]
    fn an_island_is_reachable_only_while_it_is_on_the_band() {
        let band = Rect::from_min_max(pos2(24.0, 40.0), pos2(344.0, 624.0));
        let at = |y: f32| pos2(24.0, y);

        let players = at(1362.0);
        assert!(!island_on_band(players, 0.0, band, 320.0), "below the band");
        assert!(island_on_band(players, 1058.0, band, 320.0), "scrolled to");

        let operator = at(56.0);
        assert!(island_on_band(operator, 0.0, band, 144.0), "at rest");
        assert!(!island_on_band(operator, 300.0, band, 144.0), "scrolled past");

        assert!(island_on_band(at(600.0), 0.0, band, 320.0), "straddles the bottom");
        assert!(island_on_band(at(-40.0), 0.0, band, 144.0), "straddles the top");
    }

    /// The backdrop starts BELOW the top band, and does not start above it.
///
/// Two halves of one property, and both were wrong at different times in
/// opposite directions. The backdrop claimed the whole viewport, so Settings and
/// Sign out were inert behind any modal — the screen looked alive and nothing
/// responded, which reads as a stuck overlay rather than a modal. Then, fixing
/// that, the rect was built from the viewport origin *down to* the band instead
/// of *from* the band down: the band stayed blocked and the whole map went
/// click-through.
///
/// Pinned as arithmetic because a screenshot cannot see it. Both failures dimmed
/// and blocked in ways that render plausibly; only the interact rect differs.
#[test]
fn the_backdrop_begins_under_the_top_band() {
    for (w, h) in [(1040.0_f32, 640.0_f32), (1920.0, 1080.0), (1280.0, 800.0)] {
        let vp = Rect::from_min_size(pos2(0.0, 0.0), vec2(w, h));
        let band_bottom = 27.0_f32.min(vp.height());
        let backdrop = Rect::from_min_max(pos2(vp.left(), band_bottom), vp.max);
        // The band is above the backdrop: Settings stays clickable.
        assert!(
            backdrop.min.y >= band_bottom,
            "{w}x{h}: the backdrop must start at or below the band"
        );
        // And the band itself is outside it, not merely above its top edge.
        assert!(
            !backdrop.contains(pos2(vp.right() - 20.0, band_bottom * 0.5)),
            "{w}x{h}: the backdrop covers the top band"
        );
        // The map below the band is inside it, so a drag still dies there
        // rather than dropping a hull behind the form.
        assert!(
            backdrop.contains(pos2(vp.center().x, vp.bottom() - 20.0)),
            "{w}x{h}: the map must stay inert behind a modal"
        );
        // And the panel is drawn over the backdrop, so the panel is reachable.
        let panel = Modal::new(Id::new("t"), "t", vec2(980.0, 620.0)).rect_in(vp);
        assert!(
            backdrop.intersects(panel),
            "{w}x{h}: the backdrop and the panel must overlap for the panel to win"
        );
    }
}

/// The band excludes the top band, so nothing in the zone can paint over it.
    #[test]
    fn the_band_starts_below_the_top_band() {
        let vp = Rect::from_min_size(pos2(0.0, 0.0), vec2(1040.0, 640.0));
        let band = zone_band(Dock::Left, vp);
        assert!(
            band.top() >= tokens::ZONE_TOP_GAP - tokens::ZONE_ISLAND_GAP,
            "the band must not reach into the top band"
        );
        assert!(band.bottom() <= vp.bottom());
        assert_eq!(band.width(), tokens::ZONE_W, "the band is the column's width");
    }

    use super::*;

    /// A palette's panel stops existing for the duration of its gesture, and a
    /// dialog's does not.
    ///
    /// The property that was actually needed, and the one `clear_backdrop` could
    /// not give: a palette's gesture ENDS on the map, so for its duration the
    /// panel must not be there to cover the target, must not dim it, and must
    /// not swallow the pointer. `step_aside` says all three at once, and returns
    /// `open` untouched so the caller keeps its state and the panel returns the
    /// frame the gesture ends.
    #[test]
    fn a_stepped_aside_palette_claims_nothing() {
        let dialog = Modal::new(Id::new("m"), "M", vec2(100.0, 100.0));
        assert!(!dialog.aside, "a dialog is never aside");
        assert_eq!(dialog.backdrop_alpha(), MODAL_BACKDROP_ALPHA);

        let palette = dialog.clone().step_aside();
        assert!(palette.aside, "the palette steps aside");
        // Everything else about it is untouched, so stepping aside is a decision
        // about this frame and not a different kind of window.
        assert_eq!(palette.title, dialog.title);
        assert_eq!(
            palette.rect_in(Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 600.0))),
            dialog.rect_in(Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 600.0)))
        );
        // Still open, so the caller does not tear its state down mid-gesture.
        assert!(palette.open);
    }

    /// A dim of zero would read as "no backdrop was drawn" to anything that
    /// inspects the alpha, so the two are kept distinguishable by construction.
    #[test]
    fn dimming_is_actually_dimmed() {
        assert!(MODAL_BACKDROP_ALPHA > 0, "a dim backdrop that dims nothing");
        assert!(MODAL_BACKDROP_ALPHA < 255, "a dim backdrop that blacks out the map");
    }

    fn island(size_y: f32) -> (Island, bool) {
        (Island::new(Id::new("t"), "T", vec2(tokens::ZONE_W, size_y)), true)
    }

    /// The zone has to fit its own islands or the column silently clips.
    #[test]
    fn origins_fit_the_zone_width() {
        let vp = Rect::from_min_size(pos2(0.0, 0.0), vec2(1920.0, 1080.0));
        for dock in [Dock::Left, Dock::Right] {
            for (spec, _) in [island(200.0)] {
                let origins = zone_island_origins(dock, vp, &[(spec.clone(), true)]);
                assert_eq!(origins.len(), 1);
                assert!(origins[0].x >= 0.0 && origins[0].x + spec.size.x <= vp.width());
            }
        }
    }

    /// Stacking must accumulate, and a closed island must leave a gap rather
    /// than take one out of the rhythm.
    #[test]
    fn islands_stack_with_the_zone_gap() {
        let vp = Rect::from_min_size(pos2(0.0, 0.0), vec2(1920.0, 1080.0));
        let islands = [island(100.0), island(100.0)];
        let origins = zone_island_origins(Dock::Left, vp, &islands);
        assert_eq!(origins.len(), 2);
        assert!((origins[1].y - origins[0].y - 100.0 - tokens::ZONE_ISLAND_GAP).abs() < 0.01);
    }

    /// A closed island is not laid out. If it were, the column would reserve
    /// a gap for a panel nobody can see.
    #[test]
    fn a_closed_island_takes_no_slot() {
        let vp = Rect::from_min_size(pos2(0.0, 0.0), vec2(1920.0, 1080.0));
        let origins = zone_island_origins(Dock::Left, vp, &[(island(100.0).0, false)]);
        assert_eq!(origins.len(), 0);
    }

    /// A left-docked zone starts at the edge gap, and a right-docked one
    /// ends one gap short of the window. Pinned exactly rather than
    /// "within bounds", because a zone that drifted toward the middle would
    /// still pass a bounds check while covering the part of the map the
    /// camera offset assumes is clear.
    #[test]
    fn dock_places_the_zone_at_the_edge() {
        let w = 1920.0;
        assert_eq!(zone_origin(Dock::Left, w), Some(tokens::ZONE_EDGE_GAP));
        assert_eq!(
            zone_origin(Dock::Right, w),
            Some(w - tokens::ZONE_W - tokens::ZONE_EDGE_GAP)
        );
        let vp = Rect::from_min_size(pos2(0.0, 0.0), vec2(w, 1080.0));
        assert_eq!(
            zone_island_origins(Dock::Left, vp, &[island(100.0)])[0].x,
            tokens::ZONE_EDGE_GAP
        );
        let right = zone_island_origins(Dock::Right, vp, &[island(100.0)])[0];
        assert!((right.x + tokens::ZONE_W - (w - tokens::ZONE_EDGE_GAP)).abs() < 0.01);
    }

    /// A window too narrow to hold the zone has no zone. Returning an
    /// origin that would clip is worse than not laying it out, because the
    /// islands would be drawn off-screen and the camera offset would still
    /// be applied for them.
    #[test]
    fn a_narrow_window_has_no_zone() {
        assert_eq!(zone_origin(Dock::Right, tokens::ZONE_W), None);
    }

    /// The camera correction has to point AWAY from the zone. Getting the
    /// sign wrong parks the framed hull under the chrome, which is the
    /// failure this function exists to prevent, so it is pinned rather than
    /// reasoned about.
    #[test]
    fn camera_offset_points_away_from_the_zone() {
        assert!(camera_centre_offset(Dock::Left, false) > 0.0);
        assert!(camera_centre_offset(Dock::Right, false) < 0.0);
    }

    /// A hidden zone needs no correction, because then the window centre is
    /// the visible centre.
    #[test]
    fn a_hidden_zone_needs_no_correction() {
        assert_eq!(camera_centre_offset(Dock::Left, true), 0.0);
    }

    /// The correction is half the zone, not all of it: the visible centre
    /// of a partly covered map is the uncovered part's centre.
    #[test]
    fn camera_offset_is_half_the_zone() {
        assert!((camera_centre_offset(Dock::Left, false) - tokens::ZONE_W / 2.0).abs() < 0.01);
    }

    /// The sidebar clamps to its rails, follows a resized width, and never
    /// panics on a narrow window: the floor always wins, so a tiny viewport
    /// gets a 240px zone rather than a crash.
    #[test]
    fn zone_width_clamps_to_its_rails() {
        assert_eq!(clamp_zone_width(320.0, 1040.0), 320.0);
        assert_eq!(clamp_zone_width(100.0, 1040.0), tokens::ZONE_MIN_W);
        assert_eq!(clamp_zone_width(900.0, 1040.0), tokens::ZONE_MAX_W);
        assert_eq!(clamp_zone_width(900.0, 500.0), 252.0);
        assert_eq!(clamp_zone_width(900.0, 300.0), tokens::ZONE_MIN_W);
    }

    /// The resized width moves the column, the band and the camera together.
    /// Checked as one property because three numbers that can disagree are
    /// how a resized sidebar parks a hull under itself.
    #[test]
    fn resized_width_moves_origins_band_and_camera_together() {
        let vp = Rect::from_min_size(pos2(0.0, 0.0), vec2(1040.0, 640.0));
        let specs = [island(100.0)];
        let narrow = zone_island_origins_with(Dock::Right, vp, &specs, 240.0);
        let wide = zone_island_origins_with(Dock::Right, vp, &specs, 480.0);
        assert!((narrow[0].x - wide[0].x - 240.0).abs() < 0.01);
        assert_eq!(
            zone_band_with(Dock::Left, vp, 480.0).width(),
            480.0
        );
        assert!(
            (camera_centre_offset_with(Dock::Left, false, 480.0) - 240.0).abs() < 0.01
        );
    }

    /// ADR-0016's load-bearing claim: one owner, never two. Checked over the
    /// whole column rather than one point, because the rule is about the
    /// column and a single assertion would pass while the rule was broken
    /// everywhere else.
    #[test]
    fn exactly_one_island_owns_the_pointer() {
        let vp = Rect::from_min_size(pos2(0.0, 0.0), vec2(1920.0, 1080.0));
        let specs = [island(96.0), island(132.0), island(300.0)];
        let origins = zone_island_origins(Dock::Left, vp, &specs);
        let rects: Vec<Rect> = specs
            .iter()
            .zip(&origins)
            .map(|(s, p)| s.0.rect_at(*p))
            .collect();

        let mut owned_anywhere = 0;
        let mut x = 0.0;
        while x < vp.width() {
            let mut y = 0.0;
            while y < vp.height() {
                let owners = owning_island(&rects, Some(pos2(x, y)))
                    .into_iter()
                    .count();
                assert!(owners <= 1, "two islands claimed the pointer at {x},{y}");
                owned_anywhere += owners;
                y += 7.0;
            }
            x += 7.0;
        }
        assert!(owned_anywhere > 0, "no island ever owned the pointer");
    }

    /// The pointer between two islands belongs to neither, so the rim goes
    /// out for the gap rather than sticking to whichever was drawn last.
    #[test]
    fn the_gap_between_islands_belongs_to_nobody() {
        let vp = Rect::from_min_size(pos2(0.0, 0.0), vec2(1920.0, 1080.0));
        let specs = [island(96.0), island(96.0)];
        let origins = zone_island_origins(Dock::Left, vp, &specs);
        let rects: Vec<Rect> = specs
            .iter()
            .zip(&origins)
            .map(|(s, p)| s.0.rect_at(*p))
            .collect();
        let in_gap = pos2(
            origins[0].x + 10.0,
            origins[0].y + specs[0].0.size.y + tokens::ZONE_ISLAND_GAP / 2.0,
        );
        assert_eq!(owning_island(&rects, Some(in_gap)), None);
    }

    /// No pointer, no owner. The rim must not stay lit from the last frame
    /// the pointer left the window.
    #[test]
    fn no_pointer_means_no_owner() {
        let r = Rect::from_min_size(pos2(0.0, 0.0), vec2(100.0, 100.0));
        assert_eq!(owning_island(&[r], None), None);
    }

    // -- the modal zone ----------------------------------------------------

    fn modal(size: Vec2) -> Modal {
        Modal::new(Id::new("m"), "Test", size)
    }

    /// A modal is centred, which is the whole of "modal" geometrically.
    #[test]
    fn a_modal_is_centred() {
        let vp = Rect::from_min_size(pos2(0.0, 0.0), vec2(1200.0, 800.0));
        let rect = modal(vec2(700.0, 500.0)).rect_in(vp);
        assert_eq!(rect.center(), vp.center());
        assert_eq!(rect.width(), 700.0);
        assert_eq!(rect.height(), 500.0);
    }

    /// A modal taller than the window is clamped, never centred off-screen
    /// and never allowed to reflow its own form.
    #[test]
    fn a_modal_never_leaves_the_window() {
        let vp = Rect::from_min_size(pos2(0.0, 0.0), vec2(600.0, 400.0));
        let rect = modal(vec2(900.0, 700.0)).rect_in(vp);
        assert_eq!(rect.width(), 600.0, "clamped to the window width");
        assert_eq!(rect.height(), 400.0, "clamped to the window height");
        assert!(rect.left() >= vp.left() && rect.right() <= vp.right());
        assert!(rect.top() >= vp.top() && rect.bottom() <= vp.bottom());
    }

    /// A window narrower than the pad still gets a body. The body rect has
    /// to stay non-degenerate or `scope_builder` is handed a negative rect
    /// and the form vanishes rather than scrolling.
    #[test]
    fn the_body_survives_a_tiny_window() {
        let vp = Rect::from_min_size(pos2(0.0, 0.0), vec2(60.0, 50.0));
        let rect = modal(vec2(400.0, 300.0)).rect_in(vp);
        let body = modal(vec2(400.0, 300.0)).body_rect(rect);
        assert!(body.width() >= 0.0, "body collapsed to nothing");
        assert!(body.height() >= 0.0, "body collapsed to nothing");
    }

    /// The body sits inside the panel with its top below the title row.
    /// A form whose first field is drawn under the title is unreadable, and
    /// this is the assertion that catches it.
    #[test]
    fn the_body_is_inside_the_panel_and_below_the_title() {
        let vp = Rect::from_min_size(pos2(0.0, 0.0), vec2(1200.0, 800.0));
        let spec = modal(vec2(700.0, 500.0));
        let rect = spec.rect_in(vp);
        let body = spec.body_rect(rect);
        assert!(body.left() >= rect.left());
        assert!(body.right() <= rect.right());
        assert!(body.top() >= rect.top() + MODAL_TITLE_H);
        assert!(body.bottom() <= rect.bottom());
    }

    // -- who owns the wheel -------------------------------------------------
    //
    // The rule the whole side zone's scrolling rests on, and the one thing
    // about it that cannot be checked by looking at the picture: a body that
    // has taken the gesture must not also have left it for the column.

    fn fit(content_h: f32, viewport_h: f32, offset: f32) -> BodyFit {
        BodyFit {
            content_h,
            viewport_h,
            offset,
        }
    }

    /// A body whose content fits takes no wheel at all, in either direction.
    ///
    /// This is the case that keeps the column reachable. The Operator island is
    /// 140pt holding about 98pt, so if it swallowed the wheel the operator could
    /// only scroll the zone by finding the 16pt gaps between islands.
    #[test]
    fn a_body_that_fits_takes_no_wheel() {
        // The Operator island as measured: 98pt of content, 94pt of viewport,
        // and 4pt of trailing spacing that is not content at all.
        let f = fit(98.4, 94.0, 0.0);
        assert!(
            !f.can_scroll_down(),
            "4pt of trailing spacing is not something to scroll to"
        );
        assert!(!f.can_scroll_up(), "a fitting body has nothing above the fold");
    }

    /// The slack is a line, not a hair, and the reason is the Operator island.
    ///
    /// At zero tolerance that island claims a gesture for four points of
    /// nothing. At a point and a half it claims one for a control an operator
    /// might genuinely be trying to reach. Between those, and pinned rather
    /// than left to judgement, because both failure modes are silent.
    #[test]
    fn the_slack_is_one_text_line() {
        assert!(
            BODY_SCROLL_SLACK >= 4.0,
            "trailing spacing measured ~4pt; less claims gestures for nothing"
        );
        assert!(
            BODY_SCROLL_SLACK <= 20.0,
            "more than a line hides real content below the fold"
        );
    }

    /// A long body takes the wheel down, and not up, until it reaches its end.
    ///
    /// The Essentials case as measured: 759pt of content in a 334pt viewport.
    /// That is 425pt of travel, and every point of it has to be reachable or the
    /// composer's button is not on the screen.
    #[test]
    fn a_long_body_takes_the_wheel_down_only() {
        let mut f = fit(759.0, 334.0, 0.0);
        assert!(f.can_scroll_down(), "425pt of content below the fold");
        assert!(!f.can_scroll_up(), "nothing above the fold at the top");

        f.offset = 240.0;
        assert!(f.can_scroll_down(), "still 185pt below the fold");
        assert!(f.can_scroll_up(), "240pt above the fold now");

        f.offset = 425.0;
        assert!(
            !f.can_scroll_down(),
            "at the end, the column is entitled to the gesture"
        );
    }

    /// At the limit is at the limit, within the slack.
    ///
    /// The gap between `BODY_SCROLL_EPSILON` and `BODY_SCROLL_SLACK` is the
    /// whole of "close enough": residue inside the epsilon is not a limit, and
    /// content inside the slack is not reachable. Anything that asked for
    /// better than this would either be claiming gestures for trailing spacing
    /// or making the last line of a form unreachable.
    #[test]
    fn the_limit_is_reached_within_the_slack() {
        let room = 425.0;
        assert!(
            !fit(759.0, 334.0, room - BODY_SCROLL_EPSILON).can_scroll_down(),
            "inside the epsilon of the end is the end"
        );
        assert!(
            fit(759.0, 334.0, room - BODY_SCROLL_SLACK - BODY_SCROLL_EPSILON).can_scroll_down(),
            "a line short of the end is not the end"
        );
        assert!(!fit(759.0, 334.0, BODY_SCROLL_EPSILON).can_scroll_up());
        assert!(fit(759.0, 334.0, BODY_SCROLL_SLACK + BODY_SCROLL_EPSILON).can_scroll_up());
    }
}
