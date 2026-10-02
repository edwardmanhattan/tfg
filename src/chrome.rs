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

#[cfg(test)]
mod tests {
    use super::*;

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
}
