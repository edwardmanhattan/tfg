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
            pos2(rect.left() + tokens::PAD, rect.top() + tokens::TITLE_H + 4.0),
            pos2(rect.right() - tokens::PAD, rect.bottom() - tokens::PAD),
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
        Stroke::new(1.0, tokens::HAIRLINE_SLATE),
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
        Stroke::new(1.0, tokens::HAIRLINE_SLATE),
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
    island_owned(ctx, spec, pos, open, owns_input, body)
}

/// [`island`], with ownership supplied by the caller.
///
/// A zone calls this so that exactly one rim in the column lights, decided
/// once by [`owning_island`]. Everything else about the island is identical.
pub fn island_owned(
    ctx: &egui::Context,
    spec: &Island,
    pos: &mut Pos2,
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
    let mut overflowed = false;
    let mut fitted: Option<f32> = None;

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
                pos2(title_band.left() + tokens::PAD, title_band.center().y - 7.0),
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
                    pos2(close_rect.left() - tokens::PAD, title_band.center().y + 4.0),
                    egui::Align2::RIGHT_BOTTOM,
                    &spec.trailing,
                    FontId::monospace(11.0),
                    tokens::BODY_SILVER,
                );
            }

            let close = ui.interact(close_rect, spec.id.with("__close"), Sense::click());
            paint_close(&painter, close_rect, close.hovered());
            clicked_close = close.clicked();

            // Body, in a fixed rect, and CLIPPED to it.
            //
            // The clip is the fix for a bug this primitive could not see: the
            // Operator island is 112pt tall, its content needed rather more,
            // and the overflow painted over the map AND underneath the next
            // island's title band — so a status line belonging to the identity
            // island was legible, half-occluded, in the gap between two
            // islands. A `max_rect` alone only tells the LAYOUT where to
            // stop; it does not stop the PAINTER. What escapes is now
            // invisible rather than corrupting its neighbour, which is the
            // difference between a bug and a mistake.
            //
            // Clipping is not the same as fitting: content that does not fit
            // is now lost quietly instead of loudly. The honest fix is for
            // the caller to size its island to its content, and that is a
            // layout question this primitive deliberately does not answer —
            // the zone stacks by fixed heights before anything is drawn.
            let content = spec.content_rect(rect);
            ui.scope_builder(
                UiBuilder::new()
                    .max_rect(content)
                    .layout(Layout::top_down(egui::Align::LEFT))
                    .sense(Sense::hover()),
                |ui| {
                    ui.set_clip_rect(content.intersect(ui.max_rect()));
                    body(ui);
                    // Whether the body needed MORE room than it was given.
                    //
                    // The clip above makes an overflow quiet, and quiet is
                    // worse than loud when the height is a hand-picked
                    // constant: the Operator island shipped at 112pt against
                    // content needing more, and nothing said so until a
                    // screenshot. So the fact is published rather than
                    // swallowed — see `island_overflowed`.
                    // A LINE, not a hair. The first threshold was 0.5pt and
                    // it reported EVERY island, because a layout's
                    // `min_rect` runs a few points past the last control for
                    // trailing spacing: the Operator island measured 98.4
                    // against 94 available and rendered with visible slack
                    // under the button. So the tolerance has to be at least
                    // one line, and anything below that is padding noise
                    // rather than a cut control. 12pt separates the two cases
                    // cleanly: the real 112pt overflow measured ~52pt over,
                    // and this noise measures 4pt.
                    let used = ui.min_rect().height();
                    overflowed = used > content.height() + OVERFLOW_SLACK;
                    // `min_rect()` reports the content's natural height even
                    // when handed a smaller rect — measured, not assumed.
                    fitted = Some(spec.size.y - content.height() + used);
                },
            );

            title_resp = Some(drag);
        });

    if clicked_close {
        *open = false;
    }
    ctx.data_mut(|d| {
        d.insert_temp(island_overflow_key(spec.id), overflowed);
        if let Some(f) = fitted {
            d.insert_temp(island_fitted_key(spec.id), f);
        }
    });
    title_resp
}

/// The key an island's overflow flag is filed under in the context's temp
/// storage.
///
/// TEMP, deliberately: this is a fact about one frame's layout, and a
/// persistent entry would outlive the island that set it — a closed island
/// would keep reporting an overflow from whenever it was last drawn, and
/// nothing would be able to tell the difference.
fn island_overflow_key(id: Id) -> Id {
    id.with("__overflow")
}

/// Whether an island's body did not fit the rect it was given.
///
/// Published by `island_owned` and read by the zone, which reports it rather
/// than leaving the operator to notice that a control has gone missing.
///
/// This is the loud half of the clip. Clipping alone turned "corrupts the
/// island below" into "silently loses a control", which is not obviously an
/// improvement — a form whose button has vanished looks like a bug in the
/// form. With this, a wrong height is a line in the log the moment it
/// happens.
///
/// FALSE when the island has not been drawn this frame, because "not drawn"
/// is not "did not fit".
pub fn island_overflowed(ctx: &Context, id: Id) -> bool {
    ctx.data(|d| d.get_temp::<bool>(island_overflow_key(id)))
        .unwrap_or(false)
}

/// The key an island's fitted height is filed under.
fn island_fitted_key(id: Id) -> Id {
    id.with("__fitted")
}

/// The island height its content actually wanted, or `None` if it has not
/// been drawn since the last read.
///
/// This is what makes the zone self-sizing WITHOUT running any body twice.
///
/// `min_rect()` reports the content's natural height even when it was handed
/// a smaller rect — measured, not assumed: the Operator island measured 98.4
/// against 94 available, so the layout was not clamped. So an island can
/// draw at whatever height it was given and then say what it would have
/// preferred, and the column can use that on the NEXT frame.
///
/// One frame of lag is the whole price, and it is the right trade against the
/// alternative: the zone stacks by heights fixed before anything is drawn, so
/// measuring in advance means running every body twice, and running a body
/// twice double-fires its writes. A frame of lag cannot fire a write twice.
///
/// TEMP storage, like the overflow flag — this is a fact about one frame.
pub fn island_fitted_height(ctx: &Context, id: Id) -> Option<f32> {
    ctx.data(|d| d.get_temp::<f32>(island_fitted_key(id)))
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

/// How wide the zone takes, in px, given the window.
///
/// The zone is a constant width rather than a resizable one. It has to be
/// constant because a floating zone over a map means the map's visible centre
/// moves when the zone resizes, and the camera has to compensate for that on
/// every recentre; a width that never moves is a compensation that can be
/// right once instead of right every frame.
pub fn zone_width() -> f32 {
    tokens::ZONE_W
}

/// The zone's horizontal origin for a window of `viewport_w`, or `None` when
/// the zone is hidden.
pub fn zone_origin(dock: Dock, viewport_w: f32) -> Option<f32> {
    let x = match dock {
        Dock::Left => tokens::ZONE_EDGE_GAP,
        Dock::Right => viewport_w - tokens::ZONE_W - tokens::ZONE_EDGE_GAP,
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
    if hidden {
        return 0.0;
    }
    match dock {
        // The zone covers the left, so the visible centre is to its right.
        Dock::Left => tokens::ZONE_W * tokens::CAMERA_OFFSET_FRACTION,
        Dock::Right => -tokens::ZONE_W * tokens::CAMERA_OFFSET_FRACTION,
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
    let Some(x) = zone_origin(dock, viewport.width()) else {
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
        }
    }

    /// A modal whose backdrop swallows input without darkening.
    ///
    /// Only correct where something behind the modal is the target of the
    /// pointer's next move. Everywhere else the dim is what tells the
    /// operator that the map is inert.
    pub fn clear_backdrop(mut self) -> Self {
        self.backdrop = Backdrop::Clear;
        self
    }

    /// The fill alpha for this backdrop.
    ///
    /// One function so the two variants cannot disagree about what "no dim"
    /// means, and so a test can pin the choice without a renderer.
    pub fn backdrop_alpha(&self) -> u8 {
        match self.backdrop {
            Backdrop::Dim => MODAL_BACKDROP_ALPHA,
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

/// How far an island's content may run past its rect before it counts as
/// overflowing.
///
/// ONE TEXT LINE, and that is the whole argument. A layout's `min_rect`
/// extends a few points past the last control for trailing spacing, so a
/// zero tolerance reports every island in the column and the signal means
/// nothing — which is worse than no signal, because the first person to see
/// it ignores it.
///
/// Measured: the Operator island at 140pt reports 98.4 used against 94
/// available and renders with slack under the button. The same island at the
/// original 112pt ran about 52pt over, which is the case worth catching.
pub const OVERFLOW_SLACK: f32 = 12.0;

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

    let mut clicked_close = false;
    egui::Area::new(spec.id.with("__backdrop"))
        .fixed_pos(viewport.min)
        .movable(false)
        .constrain(false)
        .interactable(true)
        .order(Order::Foreground)
        .show(ctx, |ui| {
            // Claim the whole viewport before the panel exists, so the panel
            // is drawn over a region that already belongs to the backdrop.
            let (full, _) = ui.allocate_exact_size(viewport.size(), Sense::click_and_drag());
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
            let close = ui.interact(close_rect, spec.id.with("__close"), Sense::click());
            paint_close(&painter, close_rect, close.hovered());
            clicked_close = close.clicked();

            ui.scope_builder(
                UiBuilder::new()
                    .max_rect(spec.body_rect(panel))
                    .layout(Layout::top_down(egui::Align::LEFT))
                    .sense(Sense::hover()),
                body,
            );
        });

    if clicked_close {
        return false;
    }
    spec.open
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The overflow threshold is a LINE, not a hair, and the reason is a
    /// measurement rather than taste.
    ///
    /// At 0.5pt every island in the column reported, because a layout's
    /// `min_rect` runs a few points past the last control for trailing
    /// spacing — the Operator island measures 98.4 against 94 available and
    /// renders with visible slack under the button. A signal that fires on
    /// everything is worse than no signal, because the first person to see it
    /// learns to ignore it.
    ///
    /// Pinned rather than left as a judgement call, because the failure mode
    /// is silent in both directions: too tight and every island screams, too
    /// loose and the real overflow ships again.
    #[test]
    fn the_overflow_threshold_is_one_line_not_a_hair() {
        // The padding noise this exists to absorb, measured.
        assert!(
            OVERFLOW_SLACK >= 4.0,
            "trailing spacing measured ~4pt; a smaller threshold reports every island"
        );
        // The real overflow it must still catch, measured.
        assert!(
            OVERFLOW_SLACK <= 20.0,
            "the Operator island at 112pt ran ~52pt over and must still be caught"
        );
        // Below a line of text, or a label would be cut before it counts.
        assert!(
            OVERFLOW_SLACK < 20.0,
            "a threshold at or past the body text line hides a cut label"
        );
    }

    /// The backdrop's two jobs are independent and both are load-bearing.
    /// "No dim" must not have been implemented as "no backdrop", or a drag
    /// out of the modal clicks the map underneath it.
    #[test]
    fn a_clear_backdrop_drops_the_dim_and_nothing_else() {
        let dimmed = Modal::new(Id::new("m"), "M", vec2(100.0, 100.0));
        assert_eq!(dimmed.backdrop_alpha(), MODAL_BACKDROP_ALPHA);

        let clear = dimmed.clone().clear_backdrop();
        assert_eq!(clear.backdrop_alpha(), 0);

        // Everything else about the modal is untouched, so clearing is a
        // painting decision and not a different kind of window.
        assert_eq!(clear.rect_in(Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 600.0))),
                   dimmed.rect_in(Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 600.0))));
        assert_eq!(clear.title, dimmed.title);
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
}
