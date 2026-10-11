//! Throwaway prototype: can epaint draw variant B's instrument frame?
//!
//! Six claims, each falsifiable by looking at the output. No wgpu pipeline,
//! no shader, no new `Shape` variant — the question is how far the
//! primitives that already ship get.
//!
//! 1. **The notch.** `CornerRadius` is `{nw, ne, sw, se}` and all four are
//!    `u8`, so it draws per-corner ARCS and nothing else. A diagonal cut is
//!    not expressible as a radius. It IS expressible as a `Vec<Pos2>`, and
//!    that is the entire question ADR-0014 left open.
//! 2. **The scanline band** at DESIGN.md's own `BAND_SCAN_PITCH` and
//!    `BAND_SCAN_ALPHA`, from one 1×4 `LINEAR_REPEAT` texture rather than a
//!    loop of line segments.
//! 3. **The steel bevel** as nested polygon fills, concentric because each
//!    ring regrows its own radius.
//! 4. **Corner brackets** as eight line segments.
//! 5. **A soft blurred shadow.** Two answers are built and shown side by
//!    side, because they fail differently: epaint's `Shadow` /
//!    `RectShape::blur_width` is a REAL blur but is rect-only, so it cannot
//!    follow the cut; concentric polygon fills match the silhouette exactly
//!    but are a hand-rolled falloff.
//! 6. **The lit rim** on exactly one panel, per ADR-0016.
//!
//! Everything reduces to a `Vec<Pos2>` plus a fill, which is the port.

use eframe::egui;
use egui::{
    Color32, ColorImage, Context, CornerRadius, Painter, Pos2, Rect, Shape, Stroke, TextureHandle,
    TextureOptions, pos2, vec2,
};
use std::f32::consts::PI;

use crate::tokens;

/// The accent, single-sourced from the app's own token.
///
/// This was `#5B8DEF`, the mock's blue, and it was the THIRD accent in one
/// binary alongside `tokens::RADAR_CYAN` and a typo-drifted copy of it in the
/// p5 pass. An operator cannot learn "this colour means yours" while another
/// pass in the same window paints a different one, so the prototype holds the
/// shipped hue. The mock's warm surfaces are untouched by this; only the
/// signal colour moves.
pub const ACCENT: Color32 = tokens::RADAR_CYAN;

// ---------------------------------------------------------------------------
// Silhouette
// ---------------------------------------------------------------------------

/// Sample an arc as `steps + 1` points, inclusive of both ends.
///
/// Angles are in screen space (y down), so `PI` is left and `1.5 * PI` is up.
fn arc(cx: f32, cy: f32, radius: f32, a0: f32, a1: f32, steps: usize) -> Vec<Pos2> {
    (0..=steps)
        .map(|i| {
            let t = i as f32 / steps as f32;
            let a = a0 + (a1 - a0) * t;
            pos2(cx + radius * a.cos(), cy + radius * a.sin())
        })
        .collect()
}

/// A panel whose top-right corner is CUT, not rounded.
///
/// Walks clockwise: top-left arc, top edge, the cut diagonal, right edge,
/// bottom-right arc, bottom edge, bottom-left arc.
///
/// This is the function `CornerRadius` cannot produce. A `u8` radius is an
/// arc; the thing the instrument frame wants at that corner is a straight
/// 22px diagonal, and a diagonal is a line, not a curve.
pub fn notch_polygon(rect: Rect, radius: f32, cut: f32) -> Vec<Pos2> {
    let r = radius.max(0.0);
    // The cut cannot exceed half of either side, or the panel inverts.
    let cut = cut.clamp(0.0, (rect.width().min(rect.height()) * 0.5).max(0.0));
    let mut v = Vec::new();

    // Top-left arc, from the left edge up to the top edge.
    v.extend(arc(rect.left() + r, rect.top() + r, r, PI, 1.5 * PI, 5));
    // Top edge, stopping short of the cut.
    v.push(pos2(rect.right() - cut, rect.top()));
    // The cut itself.
    v.push(pos2(rect.right(), rect.top() + cut));
    // Right edge, down to the bottom-right arc.
    v.push(pos2(rect.right(), rect.bottom() - r));
    v.extend(arc(rect.right() - r, rect.bottom() - r, r, 0.0, PI / 2.0, 5));
    // Bottom edge.
    v.push(pos2(rect.left() + r, rect.bottom()));
    // Bottom-left arc.
    v.extend(arc(rect.left() + r, rect.bottom() - r, r, PI / 2.0, PI, 5));
    v
}

// ---------------------------------------------------------------------------
// Paint
// ---------------------------------------------------------------------------

/// Which shadow answer is on screen.
#[derive(Clone, Copy, PartialEq)]
pub enum ShadowMode {
    /// epaint's `Shadow` / `blur_width`. Real blur, rect-only.
    Epaint,
    /// Concentric polygon fills. Matches the cut, hand-rolled falloff.
    Polygon,
}

/// Paint one instrument-frame panel.
pub fn paint_instrument_panel(
    painter: &Painter,
    rect: Rect,
    title: &str,
    body: &str,
    owns_input: bool,
    shadow_mode: ShadowMode,
    ctx: &Context,
    // Gate on the same key the shipped harness already uses. The rust
    // hit-rect is lab output, and a lab artifact in the default screenshot
    // makes the direction read as a broken build.
    debug: bool,
) {
    // --- The shadow, three layers, warm-tinted ---------------------------
    // The mock spends `0 20px 48px rgba(0,0,0,.5), 0 2px 6px rgba(0,0,0,.35)`
    // plus `inset 0 1px 0 rgba(237,230,218,.05)`. Three separate things, and
    // the warmth is the point: a neutral black shadow on a warm ground reads
    // as dirt, so the cast carries a trace of the room.
    //
    // The mock's shadow is also NOT pure black. It is a warm near-black,
    // which is why it sits under the panels instead of punching holes.
    const CAST: Color32 = Color32::from_rgb(0x18, 0x14, 0x11);

    match shadow_mode {
        ShadowMode::Epaint => {
            let shadow = egui::epaint::Shadow {
                offset: [0, 12],
                blur: 22,
                spread: 2,
                color: CAST.gamma_multiply(0.5),
            };
            painter.add(shadow.as_shape(rect, CornerRadius::same(tokens::ISLAND_RADIUS)));
        }
        ShadowMode::Polygon => {
            // (a) A wide, faint ambient: the 48px half of the mock's shadow.
            for i in 1..=10 {
                let grow = i as f32 * 4.8;
                let a = (52.0 * (1.0 - i as f32 / 10.0).powi(2)) as u8;
                painter.add(Shape::convex_polygon(
                    notch_polygon(rect.expand(grow), 12.0 + grow, 22.0 + grow),
                    CAST.gamma_multiply(a as f32 / 255.0),
                    egui::epaint::PathStroke::NONE,
                ));
            }
            // (b) The tight contact shadow, offset down, which is what
            // actually separates the panel from the map.
            let off = vec2(0.0, 8.0);
            for i in 1..=4 {
                let grow = i as f32 * 2.0;
                let a = (110.0 * (1.0 - i as f32 / 4.0)) as u8;
                painter.add(Shape::convex_polygon(
                    notch_polygon(rect.translate(off).expand(grow), 12.0 + grow, 22.0 + grow),
                    CAST.gamma_multiply(a as f32 / 255.0),
                    egui::epaint::PathStroke::NONE,
                ));
            }
        }
    }

    // --- The bevel ---------------------------------------------------------
    // The mock's `.panel::before` box-shadow: a dark bed with a bright wire
    // centred on it, which reads as metal catching light rather than as a
    // drawn line. Nested fills, because a stacked stroke keeps one path
    // radius while its outer edge grows with its width.
    //
    // Both tones come from `tokens::STEEL_LO/STEEL_HI`. They were re-declared
    // as literals here, which is exactly the kind of copy that drifts: the
    // token and the prototype disagreed about what steel was, and neither
    // was wrong, so nobody could tell which was authoritative.
    for (grow, tone) in [
        (4.0, tokens::STEEL_LO.gamma_multiply(0.55)),
        (3.0, tokens::STEEL_LO.gamma_multiply(0.85)),
        (2.0, tokens::STEEL_HI.gamma_multiply(0.16)),
        (1.0, tokens::STEEL_HI.gamma_multiply(0.30)),
    ] {
        painter.add(Shape::convex_polygon(
            notch_polygon(rect.expand(grow), 12.0 + grow, 22.0 + grow),
            tone,
            egui::epaint::PathStroke::NONE,
        ));
    }

    // --- The body ----------------------------------------------------------
    // The mock's panel is `rgba(26,23,20,.90)` over the map: a translucent
    // fill so the ground shows through and the panel reads as a surface
    // rather than a hole punched in it.
    painter.add(Shape::convex_polygon(
        notch_polygon(rect, 12.0, 22.0),
        Color32::from_rgb(0x1A, 0x17, 0x14).gamma_multiply(0.92),
        egui::epaint::PathStroke::NONE,
    ));

    // --- The inset top light ----------------------------------------------
    // The mock's `inset 0 1px 0 rgba(237,230,218,.05)`. One pale line just
    // inside the top edge, stopping short of the corners. It is the single
    // cheapest thing that makes a flat fill read as a physical surface.
    let r = f32::from(tokens::ISLAND_RADIUS);
    painter.line_segment(
        [
            pos2(rect.left() + r, rect.top() + 1.5),
            pos2(rect.right() - r, rect.top() + 1.5),
        ],
        Stroke::new(1.0, Color32::from_rgb(0xED, 0xE6, 0xDA).gamma_multiply(0.06)),
    );

    // --- The rim -----------------------------------------------------------
    // ADR-0016: neutral at rest, the accent on the ONE panel that owns
    // input. Never both, and exactly one.
    //
    // The colour, the band width and the inset all come from `tokens::rim`,
    // which is the same call `chrome.rs::paint_rim` makes. This used to
    // paint `ACCENT` at full opacity on a 2px band at a hand-picked inset,
    // so the prototype answered ADR-0016 differently from the shipped app:
    // cyan at 50% there, blue at 100% here. The rule is about which surface
    // owns input, not about which file draws it, so there is one answer and
    // it is that one.
    let rim_inset = tokens::RIM_INSET;
    let rim_r = (f32::from(tokens::ISLAND_RADIUS) - rim_inset).max(0.0);
    painter.add(Shape::convex_polygon(
        notch_polygon(rect.shrink(rim_inset), rim_r, rim_r + 22.0 - rim_inset),
        Color32::TRANSPARENT,
        egui::epaint::PathStroke::new(tokens::RIM_BAND, tokens::rim(owns_input)),
    ));

    // --- The title band ----------------------------------------------------
    let band = Rect::from_min_size(rect.min, vec2(rect.width(), tokens::TITLE_H));
    paint_top_gradient(painter, band);
    if let Some(tex) = scanline(ctx) {
        let tiles = vec2(
            rect.width() / tokens::BAND_SCAN_PITCH,
            tokens::TITLE_H / tokens::BAND_SCAN_PITCH,
        );
        painter.add(Shape::image(
            tex.id(),
            band,
            Rect::from_min_max(pos2(0.0, 0.0), tiles.to_pos2()),
            Color32::WHITE,
        ));
    }

    // --- Corner brackets ---------------------------------------------------
    let arm = 11.0;
    let inset = 7.0;
    for (x, y, sx, sy) in [
        (rect.left() + inset, rect.top() + inset, 1.0, 1.0),
        (rect.right() - inset, rect.top() + inset, -1.0, 1.0),
        (rect.left() + inset, rect.bottom() - inset, 1.0, -1.0),
        (rect.right() - inset, rect.bottom() - inset, -1.0, -1.0),
    ] {
        let bracket = Stroke::new(1.0, tokens::STEEL_HI);
        painter.line_segment([pos2(x, y), pos2(x + arm * sx, y)], bracket);
        painter.line_segment([pos2(x, y), pos2(x, y + arm * sy)], bracket);
    }

    // Title and body are NOT painted here. `paint_panel_title` owns the
    // title band — it draws the mock's 11px tracked-caps label and its pill,
    // which this frame's `proportional(12.5)` label was not, and two labels
    // stacked in one band is worse than either. The caller paints the title
    // and the bodies after the frames so they sit above the bevel.
    //
    // `title` and `body` are still parameters because the call sites read as
    // sentences, and because an empty `body` is how a panel with no prose of
    // its own asks for none. The body text that used to live here was
    // placeholder filler ("One panel owns input, so one rim is lit."), and
    // the rail and the seat rows replaced it.
    let _ = (title, body);

    // The hit-test problem, drawn only when it is asked for: egui allocates
    // a RECT, so the notch's cut corner is clickable-but-empty. This rust
    // outline is what a click would actually land on.
    //
    // It shipped ungated, which meant the default screenshot of the Tern
    // direction carried a debug rectangle on every panel next to a key
    // legend explaining it. An honest experiment should still be honest
    // about which screenshots are lab output and which are the product.
    if debug {
        painter.rect_stroke(
            rect,
            CornerRadius::same(0),
            Stroke::new(1.0, Color32::from_rgb(0xE2, 0x68, 0x5F)),
            egui::StrokeKind::Outside,
        );
    }
}

/// A `pitch × pitch` scanline tile, cached on the context.
///
/// Two things make this work, and the first attempt of this prototype got
/// both wrong. The tile must be `pitch` texels WIDE, because a 1-texel-wide
/// texture has no horizontal duty cycle to tile. And it must be filtered
/// with `Nearest`, because `Linear` interpolates the inked row against its
/// three transparent neighbours and the pitch smooths into a gradient.
fn scanline(ctx: &Context) -> Option<TextureHandle> {
    let id = egui::Id::new("glass_mock_scanline");
    if let Some(t) = ctx.data(|d| d.get_temp::<TextureHandle>(id)) {
        return Some(t);
    }
    let pitch = tokens::BAND_SCAN_PITCH as usize;
    // Row 0 carries the ink; rows 1..pitch are transparent. That is the
    // 4px pitch and the 1px rule, both read out of DESIGN.md.
    let mut img = ColorImage::new([pitch, pitch], vec![Color32::TRANSPARENT; pitch * pitch]);
    for x in 0..pitch {
        img[(x, 0)] = Color32::from_black_alpha(tokens::BAND_SCAN_ALPHA);
    }
    let h = ctx.load_texture("band_scan", img, TextureOptions::NEAREST_REPEAT);
    ctx.data_mut(|d| d.insert_temp(id, h.clone()));
    Some(h)
}

// ---------------------------------------------------------------------------
// The ground and the hulls
// ---------------------------------------------------------------------------

/// Sample a cubic Bezier as `steps + 1` points, inclusive of both ends.
fn cubic(p0: Pos2, p1: Pos2, p2: Pos2, p3: Pos2, steps: usize) -> Vec<Pos2> {
    (0..=steps)
        .map(|i| {
            let t = i as f32 / steps as f32;
            let u = 1.0 - t;
            pos2(
                u*u*u*p0.x + 3.0*u*u*t*p1.x + 3.0*u*t*t*p2.x + t*t*t*p3.x,
                u*u*u*p0.y + 3.0*u*u*t*p1.y + 3.0*u*t*t*p2.y + t*t*t*p3.y,
            )
        })
        .collect()
}

/// The mock's own coastline, sampled from its cubic Beziers.
///
/// The mock draws the land with six cubic segments. Straight lines between the
/// same control points read as a wedge, which is what the first attempt of this
/// produced, so the curves are sampled the way the mock wrote them.
fn land_outline(s: Rect) -> Vec<Pos2> {
    let m = |x: f32, y: f32| pos2(s.left() + x, s.top() + y);
    let mut v = Vec::new();
    let segs: [([f32; 2], [f32; 2], [f32; 2], [f32; 2]); 6] = [
        ([0.0, 620.0], [160.0, 560.0], [260.0, 590.0], [380.0, 545.0]),
        ([380.0, 545.0], [470.0, 512.0], [540.0, 520.0], [610.0, 470.0]),
        ([610.0, 470.0], [660.0, 435.0], [700.0, 400.0], [720.0, 300.0]),
        ([720.0, 300.0], [742.0, 190.0], [800.0, 120.0], [900.0, 96.0]),
        ([900.0, 96.0], [1010.0, 68.0], [1120.0, 110.0], [1200.0, 210.0]),
        ([1200.0, 210.0], [1290.0, 322.0], [1360.0, 430.0], [1440.0, 500.0]),
    ];
    for (a, b, c, d) in segs {
        v.extend(cubic(m(a[0], a[1]), m(b[0], b[1]), m(c[0], c[1]), m(d[0], d[1]), 24));
    }
    v.push(m(1440.0, 900.0));
    v.push(m(0.0, 900.0));
    v
}

/// The mock's own map, ported.
///
/// The mock spends most of its visual budget here, and a bare wash is why the
/// previous pass looked empty. Sea, one landmass, a coastline, depth contours,
/// a concentration ring and place labels, all at the mock's own geometry.
pub fn paint_map_wash(painter: &Painter, screen: Rect) {
    let s = screen;
    painter.rect_filled(s, CornerRadius::same(0), Color32::from_rgb(0x11, 0x15, 0x12));

    // The landmass. One CLOSED path with a fill: the outline is concave, and
    // `convex_polygon` tessellates a concave outline as a wedge.
    let land = land_outline(s);
    painter.add(Shape::Path(egui::epaint::PathShape {
        points: land,
        closed: true,
        fill: Color32::from_rgb(0x2C, 0x36, 0x28).gamma_multiply(0.85),
        stroke: egui::epaint::PathStroke::NONE.into(),
    }));
    // The coastline, stroked as its own path so the land parts from the water.
    painter.add(Shape::Path(egui::epaint::PathShape::line(
        land_outline(s),
        egui::epaint::PathStroke::new(
            1.5,
            Color32::from_rgb(0x6E, 0x7A, 0x63).gamma_multiply(0.55),
        ),
    )));

    // Depth contours, the mock's two long curves.
    for (y0, y1, alpha) in [(0.78, 0.72, 0.26), (0.87, 0.82, 0.17)] {
        painter.line_segment(
            [
                pos2(s.left() + s.width() * 0.08, s.top() + s.height() * y0),
                pos2(s.left() + s.width() * 0.60, s.top() + s.height() * y1),
            ],
            Stroke::new(1.0, Color32::from_rgb(0x7C, 0x8A, 0x70).gamma_multiply(alpha)),
        );
    }

    // A concentration ring: three concentric ellipses, which is how a group
    // reads as an area rather than a point.
    let c = pos2(s.left() + s.width() * 0.68, s.top() + s.height() * 0.28);
    for (rx, ry, alpha) in [(180.0, 95.0, 0.30), (120.0, 62.0, 0.22), (62.0, 32.0, 0.16)] {
        painter.add(Shape::ellipse_stroke(
            c,
            vec2(rx, ry),
            Stroke::new(1.0, Color32::from_rgb(0x8A, 0x9A, 0x80).gamma_multiply(alpha)),
        ));
    }

    // Place labels, in the map's own ink.
    let label = |p: Pos2, text: &str, alpha: f32, size: f32| {
        painter.text(
            p,
            egui::Align2::LEFT_TOP,
            text,
            inter(size),
            Color32::from_rgb(0xD9, 0xE2, 0xD2).gamma_multiply(alpha),
        );
    };
    label(
        pos2(s.left() + s.width() * 0.50, s.top() + s.height() * 0.16),
        "SATUAN TUGAS BRAVO",
        0.62,
        12.0,
    );
    label(
        pos2(s.left() + s.width() * 0.76, s.top() + s.height() * 0.52),
        "LAUT JAWA",
        0.45,
        12.0,
    );
    label(
        pos2(s.left() + s.width() * 0.13, s.top() + s.height() * 0.95),
        "SELAT SUNDA",
        0.30,
        12.0,
    );
}

/// A hull. `own` is the selected one, and gets the accent ring plus a
/// reticle, so the accent can be judged over the same ground.
pub fn paint_unit(painter: &Painter, center: Pos2, own: bool) {
    let ring = if own {
        ACCENT
    } else {
        Color32::from_rgb(0x9A, 0x94, 0x88)
    };
    painter.circle_stroke(center, 11.0, Stroke::new(2.0, ring));
    if own {
        for r in [22.0, 30.0] {
            painter.circle_stroke(center, r, Stroke::new(1.0, ring.gamma_multiply(0.45)));
        }
        for a in 0..8 {
            let t = a as f32 / 8.0 * PI * 2.0;
            painter.line_segment(
                [
                    pos2(center.x + 33.0 * t.cos(), center.y + 33.0 * t.sin()),
                    pos2(center.x + 39.0 * t.cos(), center.y + 39.0 * t.sin()),
                ],
                Stroke::new(1.0, ring.gamma_multiply(0.6)),
            );
        }
    }
    painter.text(
        pos2(center.x + 16.0, center.y - 7.0),
        egui::Align2::LEFT_CENTER,
        if own { "KRI RIGEL" } else { "KRI LAMPO" },
        mono(11.0),
        Color32::from_rgb(0xDC, 0xE4, 0xDA),
    );
}

/// The title band's gradient, faked as stacked bands.
///
/// The mock uses `linear-gradient(to bottom, rgba(237,230,218,.05), transparent)`.
/// epaint has no gradient FILL, and `PathStroke::new_uv` colours a stroke, not
/// a fill. So this interpolates across `steps` horizontal bands instead. At the
/// band's 30px height, 30 bands is exact and costs nothing.
fn paint_top_gradient(painter: &Painter, band: Rect) {
    const STEPS: usize = 20;
    let h = band.height() / STEPS as f32;
    let ink = Color32::from_rgb(0xED, 0xE6, 0xDA);
    for i in 0..STEPS {
        // Ease-out, so the light concentrates just under the top edge the
        // way a real directional light does, rather than falling off evenly.
        let t = i as f32 / STEPS as f32;
        let a = 0.05 * (1.0 - t).powi(2);
        painter.rect_filled(
            Rect::from_min_size(pos2(band.left(), band.top() + i as f32 * h), vec2(band.width(), h + 1.0)),
            CornerRadius::same(0),
            ink.gamma_multiply(a),
        );
    }
}

/// A panel helper, so the caller states a panel in four numbers.
pub fn panel_rect(x: f32, y: f32, w: f32, h: f32) -> Rect {
    Rect::from_min_size(pos2(x, y), vec2(w, h))
}

// ---------------------------------------------------------------------------
// Panel content
// ---------------------------------------------------------------------------
//
// The mock's B panels each carry a whole console: Session has the lifecycle
// rail plus a ratio field and a Pause/Run pair, Roster has four seat rows with
// affiliation dots, Settings has a search field, six tabs and four cards. The
// frame pass shipped three empty shells for a long time, which is exactly why
// it read as a frame study rather than as the product.
//
// All geometry and ink below is the mock's own (`index.html:31-75` and the
// `v-instrument` board at `:271-398`), with ONE deliberate deviation: the
// mock's `--ink-low` is `#6B6357`, which lands at 2.5:1 on a panel fill this
// dark. Everything that sets 12px text uses the measured `comp::INK_LOW`
// instead, so the rows stay readable.

/// The panel's title band: the mock's 40px `.panel-title`.
///
/// Ink, size and tracking are the mock's `11px / .09em / uppercase` label.
/// The pill on the right is the mock's `.pill`, and its two kinds differ only
/// in fill: `.plan` is ink at 8%, `.live` is the signal at 15%.
pub fn paint_panel_title(
    painter: &Painter,
    rect: Rect,
    title: &str,
    pill: Option<(&str, bool)>,
) {
    let band = Rect::from_min_size(rect.min, vec2(rect.width(), tokens::TITLE_H));
    let ink = comp::INK;
    let ink_mid = comp::INK_MID;
    let cy = band.center().y;

    // The hairline under the title, drawn on top of the frame's own bevel.
    painter.line_segment(
        [pos2(band.left(), band.bottom()), pos2(band.right(), band.bottom())],
        Stroke::new(1.0, ink.gamma_multiply(0.06)),
    );
    painter.text(
        pos2(band.left() + 14.0, cy),
        egui::Align2::LEFT_CENTER,
        title,
        tracked_font(11.0),
        ink_mid,
    );

    if let Some((label, live)) = pill {
        let w = text_w(painter, label, tracked_font(11.0)) + 18.0;
        let pill_r = Rect::from_min_size(
            pos2(band.right() - 14.0 - w, cy - 10.0),
            vec2(w, 20.0),
        );
        painter.rect_filled(
            pill_r,
            CornerRadius::same(5),
            if live {
                ACCENT.gamma_multiply(0.15)
            } else {
                ink.gamma_multiply(0.08)
            },
        );
        painter.text(
            pill_r.center(),
            egui::Align2::CENTER_CENTER,
            label,
            tracked_font(11.0),
            if live { ACCENT } else { ink_mid },
        );
    }
}

// ---------------------------------------------------------------------------
// The mock's typefaces
// ---------------------------------------------------------------------------
//
// egui's default proportional family is `[Ubuntu-Light, NotoEmoji, …]` and its
// monospace family is `[Hack, UbuntuMono, …]`. The mock is Inter with JetBrains
// Mono carrying every data line, so without this the console's labels render in
// Ubuntu Light and its coordinates, unit names and clock faces render in Hack.
// Hack and JetBrains Mono are not interchangeable: Hack is a wide Vera
// derivative with a slashed zero, JetBrains Mono is taller and narrower with a
// higher x-height. On a console whose densest layer is monospace data, that is
// most of what the surface looks like.
//
// A weight is a FAMILY, not an axis. epaint's `FontId` is
// `{ size, family }` with `// TODO(emilk): weight (bold), italics, …`, so
// Inter at 400 and Inter at 500 are two registered fonts and the call site
// names the weight it wants. The mock declares 300/400/500/600 for Inter and
// 400/500 for JetBrains Mono.

/// Inter Light (300), for the mock's lightest running text.
pub const FONT_INTER_LIGHT: &str = "Inter Light";
/// Inter Regular (400), the mock's default body weight.
pub const FONT_INTER_REGULAR: &str = "Inter Regular";
/// Inter Medium (500), the mock's `.label` and `.card h4` weight.
pub const FONT_INTER_MEDIUM: &str = "Inter Medium";
/// Inter SemiBold (600), the mock's `.tag b` weight.
pub const FONT_INTER_SEMIBOLD: &str = "Inter SemiBold";
/// JetBrains Mono Regular (400), the mock's `.mono` weight.
pub const FONT_MONO_REGULAR: &str = "JetBrains Mono Regular";

fn inter(pt: f32) -> egui::FontId {
    egui::FontId::new(pt, egui::FontFamily::Name(std::sync::Arc::from(FONT_INTER_REGULAR.to_owned())))
}

fn inter_light(pt: f32) -> egui::FontId {
    egui::FontId::new(
        pt,
        egui::FontFamily::Name(std::sync::Arc::from(FONT_INTER_LIGHT.to_owned())),
    )
}

/// Inter Medium. This is the mock's `.label`: `font-weight:500`.
fn inter_medium(pt: f32) -> egui::FontId {
    egui::FontId::new(
        pt,
        egui::FontFamily::Name(std::sync::Arc::from(FONT_INTER_MEDIUM.to_owned())),
    )
}

fn inter_semibold(pt: f32) -> egui::FontId {
    egui::FontId::new(
        pt,
        egui::FontFamily::Name(std::sync::Arc::from(FONT_INTER_SEMIBOLD.to_owned())),
    )
}

fn mono(pt: f32) -> egui::FontId {
    egui::FontId::new(
        pt,
        egui::FontFamily::Name(std::sync::Arc::from(FONT_MONO_REGULAR.to_owned())),
    )
}

/// A tracked-caps `FontId`, the mock's `.label` treatment.
///
/// Tracking is expressed in points here rather than in the `extra_letter_spacing`
/// the sheet's `tracked()` uses, because `painter.text` has no `TextFormat` to
/// carry it and the mock's `.09em` at 11px is 1pt.
fn tracked_font(pt: f32) -> egui::FontId {
    inter_medium(pt)
}

/// Width of a single-line string, so a right-aligned item can be placed
/// against the panel's edge without guessing.
fn text_w(painter: &Painter, text: &str, font: egui::FontId) -> f32 {
    painter
        .layout_no_wrap(text.to_owned(), font, egui::Color32::WHITE)
        .size()
        .x
}

/// The body area a panel's content is painted into: the mock's `padding:14px`,
/// offset below the title band.
fn panel_body(rect: Rect) -> Rect {
    Rect::from_min_size(
        pos2(rect.left() + 14.0, rect.top() + tokens::TITLE_H + 14.0),
        vec2(rect.width() - 28.0, rect.height() - tokens::TITLE_H - 28.0),
    )
}

/// The Session panel: the lifecycle rail, a ratio field and a Pause/Run pair.
///
/// The rail's three steps differ by ink alone — `.done` is mid, `.now` is
/// bright and its numeral is the signal colour, `.step` (pending) is low. No
/// progress bar, no animation: state is stated, not decorated.
pub fn paint_session_body(painter: &Painter, rect: Rect) {
    let b = panel_body(rect);
    let ink = comp::INK;
    let ink_mid = comp::INK_MID;
    let ink_low = comp::INK_LOW;

    // --- The rail ---------------------------------------------------------
    let steps: [(&str, &str, f32); 3] = [
        ("1", "Setup", 0.5),
        ("2", "Live", 1.0),
        ("3", "Closed", 0.0),
    ];
    let names = ["Setup", "Live", "Closed"];
    let times = ["08:00", "14:22:07", "18:00"];
    let row_h = 30.0;
    for (i, name) in names.iter().enumerate() {
        let y = b.top() + i as f32 * row_h;
        let cy = y + row_h * 0.5;
        let now = i == 1;
        let done = i == 0;
        // The mock: `.step` is low ink, `.done` is mid, `.now` is full, and
        // `.now .n` takes the signal.
        let tone = if now {
            ink
        } else if done {
            ink_mid
        } else {
            ink_low
        };
        // The numeral, in a 16px ring of the current ink.
        let ring = Rect::from_center_size(pos2(b.left() + 8.0, cy), vec2(16.0, 16.0));
        painter.add(Shape::ellipse_stroke(
            ring.center(),
            vec2(8.0, 8.0),
            Stroke::new(1.0, if now { ACCENT } else { tone }),
        ));
        painter.text(
            ring.center(),
            egui::Align2::CENTER_CENTER,
            steps[i].0,
            mono(9.0),
            if now { ACCENT } else { tone },
        );
        painter.text(
            pos2(b.left() + 30.0, cy),
            egui::Align2::LEFT_CENTER,
            *name,
            inter(12.0),
            tone,
        );
        // The clock, pushed to the right edge.
        painter.text(
            pos2(b.right(), cy),
            egui::Align2::RIGHT_CENTER,
            times[i],
            mono(11.0),
            if now { ink_mid } else { ink_low },
        );
    }

    // --- The ratio field and the Pause/Run pair ---------------------------
    let row_y = b.top() + 3.0 * row_h + 12.0;
    let field_w = 120.0;
    let seg_w = b.width() - field_w - 10.0;
    paint_field(painter, Rect::from_min_size(pos2(b.left(), row_y), vec2(field_w, 32.0)), "× 1.0", "ratio");
    paint_segment(
        painter,
        Rect::from_min_size(pos2(b.left() + field_w + 10.0, row_y), vec2(seg_w, 32.0)),
        &["Pause", "Run"],
        1,
    );
}

/// The Roster panel: four seat rows, each with an affiliation dot.
///
/// Affiliation is the dot's colour and nothing else — live ink for the seated,
/// the signal for the commander, low ink for the observer. The rule on this
/// surface is that a rank colour never reaches the chrome, so the dots carry
/// affiliation and the chrome stays neutral.
pub fn paint_roster_body(painter: &Painter, rect: Rect) {
    let b = panel_body(rect);
    let ink = comp::INK;
    let ink_mid = comp::INK_MID;
    let ink_low = comp::INK_LOW;

    let rows: [(&str, Color32, &str, &str); 4] = [
        ("Arif", ink, "KRI Rigel", "Helm"),
        ("Budi", ink, "KRI Lampo", "Helm"),
        ("Citra", ACCENT, "Satgas Bravo", "Commander"),
        ("Dewi", ink_low, "Unsur 2", "Observer"),
    ];
    let row_h = 34.0;
    // `where` is a Rust keyword, and it is the mock's own CSS class name,
    // so the tuple field is named after the role it plays instead.
    for (i, (who, dot, unit, state)) in rows.iter().enumerate() {
        let y = b.top() + i as f32 * row_h;
        let cy = y + row_h * 0.5;
        // The mock's `.seat + .seat` hairline, so the rows read as rows.
        if i > 0 {
            painter.line_segment(
                [pos2(b.left(), y), pos2(b.right(), y)],
                Stroke::new(1.0, ink.gamma_multiply(0.045)),
            );
        }
        painter.circle_filled(pos2(b.left() + 3.0, cy), 3.0, *dot);
        painter.text(
            pos2(b.left() + 14.0, cy),
            egui::Align2::LEFT_CENTER,
            *who,
            inter(12.5),
            ink,
        );
        painter.text(
            pos2(b.left() + 14.0 + 52.0, cy),
            egui::Align2::LEFT_CENTER,
            *unit,
            inter(11.5),
            ink_low,
        );
        painter.text(
            pos2(b.right(), cy),
            egui::Align2::RIGHT_CENTER,
            *state,
            tracked_font(11.0),
            ink_low,
        );
        let _ = ink_mid;
    }
}

/// The Settings panel: search field, the tab row, and four cards.
///
/// The cards are the mock's `.card` exactly — ink at 3.5% fill with a 5%
/// hairline — and each one's control is the same `.field` / `.seg` the rails
/// use, so the Settings body and the Session body are one system.
pub fn paint_settings_body(painter: &Painter, rect: Rect) {
    let b = panel_body(rect);
    let ink = comp::INK;
    let ink_mid = comp::INK_MID;
    let ink_low = comp::INK_LOW;

    // --- Search field -----------------------------------------------------
    paint_field(
        painter,
        Rect::from_min_size(b.min, vec2(b.width(), 32.0)),
        "Search settings",
        "",
    );

    // --- The tab row -------------------------------------------------------
    // The mock's `justify-content:space-between`: each label takes its OWN
    // natural width and the leftover space becomes the gaps. Laid out in
    // fixed slots the labels overrun their slot and collide, which is what
    // the first version of this did — "Appearance" is wider than the room
    // a sixth of the width leaves it.
    let tabs = ["Appearance", "Model", "Sound", "Layout", "Input", "Motion"];
    let row_y = b.top() + 42.0;
    let tab_font = tracked_font(11.0);
    let widths: Vec<f32> = tabs
        .iter()
        .map(|t| text_w(painter, t, tab_font.clone()))
        .collect();
    let sum: f32 = widths.iter().sum();
    let gaps = (b.width() - sum) / (tabs.len() - 1) as f32;
    let mut x = b.left();
    for (i, t) in tabs.iter().enumerate() {
        painter.text(
            pos2(x, row_y),
            egui::Align2::LEFT_CENTER,
            *t,
            tab_font.clone(),
            if i == 0 { ink_mid } else { ink_low },
        );
        x += widths[i] + gaps;
    }

    // The hairline under the tabs.
    let rule_y = row_y + 14.0;
    painter.line_segment(
        [pos2(b.left(), rule_y), pos2(b.right(), rule_y)],
        Stroke::new(1.0, ink.gamma_multiply(0.07)),
    );

    // --- Four cards -------------------------------------------------------
    let cards: [(&str, &str, Control); 4] = [
        (
            "What the command bar runs",
            "Shell the command bar opens on Ctrl+Shift+A.",
            Control::Field("omp"),
        ),
        (
            "A new agent block opens",
            "Replace the tab's only pane when it is idle at a prompt, and split otherwise.",
            Control::Segments(&["Split", "Replace", "New tab"], 1),
        ),
        (
            "Text size",
            "Base the console lays out in. Settings multiplies it.",
            Control::Segments(&["Default", "Larger"], 1),
        ),
        (
            "New tabs start in",
            "Always the directory of the block they split from.",
            Control::Segments(&["Current", "Home"], 0),
        ),
    ];
    let inner_w = b.width() - 28.0;
    let mut y = rule_y + 16.0;
    for (heading, para, control) in cards.iter() {
        // The card's height comes from the paragraph's MEASURED wrapped
        // height, not from an assumed line count. A paragraph that is one
        // row on a wide card is three on a narrow one, and a card that
        // assumes clips its own copy.
        let head_h = 17.0;
        let para_h = text_wrapped(
            painter,
            pos2(0.0, 0.0),
            para,
            inter(12.0),
            ink_mid,
            inner_w,
        );
        let ctrl_y = y + 12.0 + head_h + 3.0 + para_h + 9.0;
        let card_h = ctrl_y + 32.0 + 12.0 - y;
        let card_r = Rect::from_min_size(pos2(b.left(), y), vec2(b.width(), card_h));
        painter.rect_filled(card_r, CornerRadius::same(8), ink.gamma_multiply(0.035));
        painter.rect_stroke(
            card_r,
            CornerRadius::same(8),
            Stroke::new(1.0, ink.gamma_multiply(0.05)),
            egui::StrokeKind::Inside,
        );

        painter.text(
            pos2(card_r.left() + 14.0, y + 12.0),
            egui::Align2::LEFT_TOP,
            *heading,
            // The mock's `.card h4` is `font-weight:500`, which is a family
            // here because egui has no weight axis.
            inter_medium(12.5),
            ink,
        );
        text_wrapped(
            painter,
            pos2(card_r.left() + 14.0, y + 12.0 + head_h + 3.0),
            para,
            inter(12.0),
            ink_mid,
            inner_w,
        );
        let ctrl_r = Rect::from_min_size(pos2(card_r.left() + 14.0, ctrl_y), vec2(inner_w, 32.0));
        match control {
            Control::Field(t) => paint_field(painter, ctrl_r, t, ""),
            Control::Segments(labels, on) => {
                paint_segment(painter, ctrl_r, labels, *on)
            }
        }
        y = card_r.bottom() + 8.0;
    }
}

/// What a Settings card ends with.
enum Control<'a> {
    /// A field, with its value text.
    Field(&'a str),
    /// A segmented control, its labels, and which one holds.
    Segments(&'a [&'a str], usize),
}

/// Paint wrapped text and return its measured height.
///
/// `painter.text` lays out one line and nothing else, so a paragraph wider
/// than its card runs straight out of it. The mock's `.card p` wraps at the
/// card's width; this is the same thing, a `LayoutJob` with a `max_width` and
/// no row cap.
fn text_wrapped(
    painter: &Painter,
    at: Pos2,
    text: &str,
    font: egui::FontId,
    color: Color32,
    max_w: f32,
) -> f32 {
    let mut job = egui::text::LayoutJob::default();
    job.wrap.max_width = max_w;
    job.wrap.max_rows = 0;
    job.append(
        text,
        0.0,
        egui::TextFormat {
            font_id: font,
            color,
            ..Default::default()
        },
    );
    let g = painter.layout_job(job);
    painter.galley(at, g.clone(), color);
    g.size().y
}

/// A recessed bed with a label and a value, the mock's `.field`.
fn paint_field(painter: &Painter, rect: Rect, value: &str, hint: &str) {
    let ink = comp::INK;
    let ink_low = comp::INK_LOW;
    painter.rect_filled(rect, CornerRadius::same(6), egui::Color32::BLACK.gamma_multiply(0.30));
    painter.rect_stroke(
        rect,
        CornerRadius::same(6),
        Stroke::new(1.0, ink.gamma_multiply(0.07)),
        egui::StrokeKind::Inside,
    );
    if !hint.is_empty() {
        painter.text(
            pos2(rect.left() + 11.0, rect.center().y),
            egui::Align2::LEFT_CENTER,
            hint,
            tracked_font(11.0),
            ink_low,
        );
    }
    painter.text(
        pos2(rect.right() - 11.0, rect.center().y),
        egui::Align2::RIGHT_CENTER,
        value,
        mono(12.5),
        ink,
    );
}

/// A segmented control in a recessed track, the mock's `.seg`.
///
/// The track is a bed; the held segment is raised out of it with ink at 10%
/// and a wire of light along its top edge, which is the same raised/recessed
/// relationship the rest of the console runs on.
fn paint_segment(painter: &Painter, rect: Rect, labels: &[&str], on: usize) {
    let ink = comp::INK;
    let ink_mid = comp::INK_MID;
    let ink_low = comp::INK_LOW;
    let pad = 2.0;
    let track = rect.shrink(pad);
    painter.rect_filled(track, CornerRadius::same(6), egui::Color32::BLACK.gamma_multiply(0.28));

    // The track is RECESSED: dark, an inner shadow at the top, a wire of light
    // along the bottom.
    inset_top(painter, track, 3, egui::Color32::BLACK.gamma_multiply(0.55));
    bottom_wire(painter, track, 6.0, ink.gamma_multiply(0.03));

    // Labels are laid out in the space left by the track's padding, with the
    // mock's 11px horizontal and 6px vertical button padding.
    let gap = 2.0;
    let n = labels.len() as f32;
    let inner = track.shrink(2.0);
    let total: f32 = labels
        .iter()
        .map(|l| text_w(painter, l, inter(12.0)))
        .sum();
    let avail = inner.width() - gap * (n - 1.0);
    let span = ((avail - total) / n).max(2.0);
    let mut x = inner.left();
    for (i, l) in labels.iter().enumerate() {
        let w = text_w(painter, l, inter(12.0)) + span * 2.0;
        let r = Rect::from_min_size(pos2(x, inner.top()), vec2(w, inner.height()));
        let held = i == on;
        if held {
            // RAISED: ink at 10%, and the wire of light along its top edge.
            painter.rect_filled(r, CornerRadius::same(4), ink.gamma_multiply(0.10));
            top_wire(painter, r, 4.0, ink.gamma_multiply(0.06));
        }
        painter.text(
            r.center(),
            egui::Align2::CENTER_CENTER,
            *l,
            inter(12.0),
            if held { ink } else if false { ink_mid } else { ink_low },
        );
        x += w + gap;
    }
}

// ---------------------------------------------------------------------------
// The console
// ---------------------------------------------------------------------------
//
// The mock's B layout is a whole console: a top band that says whose session
// this is, a graticule under the map, a north arrow, a scale bar, a fix
// callout, and three panels' worth of real content. The prototype had the
// frame of one panel and the furniture of none, which is why it read as a
// frame study rather than as the product. These are ported at the mock's own
// geometry so the two can be compared side by side.

/// The mock's top band: mark, breadcrumb, wire pill, game clock, seat.
///
/// Height 56px in the mock, sitting flush to the top of the window. Everything
/// in it is asymmetric on purpose — the identity on the left, the clock and
/// seat on the right, and the breadcrumb taking the slack.
pub fn paint_top_band(painter: &Painter, screen: Rect) {
    let h = 56.0;
    let band = Rect::from_min_size(screen.min, vec2(screen.width(), h));
    let ink = Color32::from_rgb(0xED, 0xE6, 0xDA);
    let ink_mid = Color32::from_rgb(0xA7, 0x9D, 0x8E);
    let ink_low = Color32::from_rgb(0x6B, 0x63, 0x57);

    // A bed, so the band is a surface and not a line of text on the map.
    painter.rect_filled(band, CornerRadius::ZERO, Color32::from_rgb(0x0E, 0x0C, 0x09).gamma_multiply(0.92));
    // And a bright wire along its bottom edge, where the band meets the map.
    painter.line_segment(
        [pos2(band.left(), band.bottom() - 0.5), pos2(band.right(), band.bottom() - 0.5)],
        Stroke::new(1.0, ink.gamma_multiply(0.10)),
    );

    let cy = band.center().y;

    // --- The mark: a rotated square with a centre dot -----------------------
    // Drawn as a quad rather than a texture, because it is four lines.
    let mark = Rect::from_center_size(pos2(band.left() + 30.0, cy), vec2(18.0, 18.0));
    painter.add(Shape::convex_polygon(
        vec![
            pos2(mark.center().x, mark.top()),
            pos2(mark.right(), mark.center().y),
            pos2(mark.center().x, mark.bottom()),
            pos2(mark.left(), mark.center().y),
        ],
        Color32::TRANSPARENT,
        egui::epaint::PathStroke::new(1.4, ACCENT),
    ));
    painter.circle_filled(mark.center(), 2.4, ACCENT);
    painter.text(
        pos2(mark.right() + 12.0, cy),
        egui::Align2::LEFT_CENTER,
        "tfg",
        inter(14.0),
        ink,
    );

    // --- The divider, then the breadcrumb -----------------------------------
    let bx = mark.right() + 12.0 + painter
        .layout_no_wrap("tfg".to_owned(), inter(14.0), ink)
        .size().x
        + 18.0;
    painter.line_segment(
        [pos2(bx, cy - 9.0), pos2(bx, cy + 9.0)],
        Stroke::new(1.0, ink.gamma_multiply(0.14)),
    );
    let crumb = "Operasi Gabungan  /  Satuan Tugas Bravo  /  KRI Rigel";
    painter.text(
        pos2(bx + 16.0, cy),
        egui::Align2::LEFT_CENTER,
        crumb,
        inter(13.0),
        ink_mid,
    );

    // --- The right side: wire pill, clock, divider, seat ---------------------
    // Laid out right-to-left from a known edge, because the clock's width is
    // the only thing here that depends on the string.
    let seat = "Helm · Lt. Kdr. Arif";
    let seat_w = painter
        .layout_no_wrap(seat.to_owned(), inter(13.0), ink_mid)
        .size().x;
    let clock = "GAME 14:22:07";
    let clock_w = painter
        .layout_no_wrap(clock.to_owned(), mono(12.5), ink_mid)
        .size().x;
    let right = band.right() - 24.0;

    painter.text(
        pos2(right, cy),
        egui::Align2::RIGHT_CENTER,
        seat,
        inter(13.0),
        ink_mid,
    );
    let div_x = right - seat_w - 18.0;
    painter.line_segment(
        [pos2(div_x, cy - 9.0), pos2(div_x, cy + 9.0)],
        Stroke::new(1.0, ink.gamma_multiply(0.14)),
    );
    painter.text(
        pos2(div_x - 16.0, cy),
        egui::Align2::RIGHT_CENTER,
        clock,
        mono(12.5),
        ink_mid,
    );

    // The wire pill: a live dot in the signal accent and the word "Wire".
    let pill_w = 62.0;
    let pill_x = div_x - 16.0 - clock_w - 18.0 - pill_w;
    painter.rect_filled(
        Rect::from_center_size(pos2(pill_x + pill_w * 0.5, cy), vec2(pill_w, 22.0)),
        CornerRadius::same(11),
        Color32::from_rgb(0x1A, 0x14, 0x10),
    );
    painter.rect_stroke(
        Rect::from_center_size(pos2(pill_x + pill_w * 0.5, cy), vec2(pill_w, 22.0)),
        CornerRadius::same(11),
        Stroke::new(1.0, ink.gamma_multiply(0.10)),
        egui::StrokeKind::Inside,
    );
    painter.circle_filled(pos2(pill_x + 12.0, cy), 3.0, ACCENT);
    painter.text(
        pos2(pill_x + 22.0, cy),
        egui::Align2::LEFT_CENTER,
        "Wire",
        inter(12.0),
        ink_low,
    );
    let _ = ink_mid;
}

/// The mock's graticule: a 9×9 lat/long grid at very low alpha.
///
/// The mock draws it as one SVG `<g>` with `opacity=".07"` over the whole
/// 1440×900 map. Two families of lines — nine vertical, nine horizontal — which
/// is why the numbers are nine and not ten.
pub fn paint_graticule(painter: &Painter, screen: Rect) {
    let ink = Color32::from_rgb(0xED, 0xE6, 0xDA);
    for i in 1..9 {
        let x = screen.left() + screen.width() * (i as f32 / 9.0);
        painter.line_segment(
            [pos2(x, screen.top()), pos2(x, screen.bottom())],
            Stroke::new(1.0, ink.gamma_multiply(0.07)),
        );
        let y = screen.top() + screen.height() * (i as f32 / 9.0);
        painter.line_segment(
            [pos2(screen.left(), y), pos2(screen.right(), y)],
            Stroke::new(1.0, ink.gamma_multiply(0.07)),
        );
    }
}

/// The north arrow: a outlined diamond arrow over a cross-line stem.
pub fn paint_north(painter: &Painter, at: Pos2) {
    let ink = Color32::from_rgb(0xD9, 0xE2, 0xD2);
    let s = vec2(26.0, 40.0);
    let r = Rect::from_center_size(at, s);
    let c = r.center();
    // The arrow head, an open diamond.
    painter.add(Shape::closed_line(
        vec![
            pos2(c.x, r.top() + 2.0),
            pos2(c.x + 5.0, r.top() + 24.0),
            pos2(c.x, r.top() + 20.0),
            pos2(c.x - 5.0, r.top() + 24.0),
        ],
        Stroke::new(1.2, ink.gamma_multiply(0.7)),
    ));
    // The stem and the crossbar.
    painter.line_segment(
        [pos2(c.x, r.top() + 20.0), pos2(c.x, r.bottom() - 4.0)],
        Stroke::new(1.0, ink.gamma_multiply(0.45)),
    );
    painter.line_segment(
        [pos2(c.x - 4.0, r.bottom() - 4.0), pos2(c.x + 4.0, r.bottom() - 4.0)],
        Stroke::new(1.0, ink.gamma_multiply(0.45)),
    );
    painter.text(
        pos2(c.x, r.top() + 12.0),
        egui::Align2::CENTER_CENTER,
        "N",
        mono(9.0),
        ink.gamma_multiply(0.75),
    );
}

/// The scale bar: a checkered bar with its caption.
///
/// Three equal cells, the first and last inked. The mock builds the checker with
/// a `linear-gradient` hard-stop split; epaint has no gradient fill, so this
/// paints the two inked cells as rects and leaves the middle one empty, which
/// is the same three cells by a different route.
pub fn paint_scale_bar(painter: &Painter, at: Pos2) {
    let ink = Color32::from_rgb(0xD9, 0xE2, 0xD2);
    let w = 132.0 / 3.0;
    let h = 5.0;
    let top = Rect::from_min_size(at, vec2(w, h));
    painter.rect_filled(top, CornerRadius::ZERO, ink.gamma_multiply(0.55));
    painter.rect_filled(
        Rect::from_min_size(pos2(at.x + w * 2.0, at.y), vec2(w, h)),
        CornerRadius::ZERO,
        ink.gamma_multiply(0.55),
    );
    // Left, bottom and right edges only — the mock sets `border-top: 0`.
    painter.line_segment([pos2(at.x, at.y + h), pos2(at.x, at.y)], Stroke::new(1.0, ink.gamma_multiply(0.45)));
    painter.line_segment(
        [pos2(at.x, at.y + h), pos2(at.x + w * 3.0, at.y + h)],
        Stroke::new(1.0, ink.gamma_multiply(0.45)),
    );
    painter.line_segment(
        [pos2(at.x + w * 3.0, at.y + h), pos2(at.x + w * 3.0, at.y)],
        Stroke::new(1.0, ink.gamma_multiply(0.45)),
    );
    painter.text(
        pos2(at.x, at.y + h + 7.0),
        egui::Align2::LEFT_TOP,
        "0 ——— 10 NM",
        mono(10.5),
        ink.gamma_multiply(0.55),
    );
}

/// The fix callout: coordinates over the game clock.
pub fn paint_callout(painter: &Painter, at: Pos2) {
    let signal = Color32::from_rgb(0x22, 0xD3, 0xEE);
    let ink_mid = Color32::from_rgb(0xA7, 0x9D, 0x8E);
    painter.text(
        at,
        egui::Align2::LEFT_TOP,
        "08°12’S 114°26’E",
        mono(11.0),
        signal.gamma_multiply(0.8),
    );
    painter.text(
        pos2(at.x, at.y + 24.0),
        egui::Align2::LEFT_TOP,
        "GAME 14:22:07 · FIX 2s AGO",
        mono(10.5),
        ink_mid.gamma_multiply(0.55),
    );
}

// ---------------------------------------------------------------------------
// The components
//
// Everything below is a REAL egui widget carrying its own state. Nothing is
// hand-painted to look like a widget, because the question "can egui render a
// crisp field" is not answered by a field I drew myself. The only thing drawn
// by hand is the frame AROUND each widget, and even that reacts to the real
// widget's `Response`.
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// The component sheet
// ---------------------------------------------------------------------------

// One light, high and slightly left. Every surface below follows from that
// single fact, so nothing here is a matter of taste:
//
//   RAISED    is lighter than its ground, carries a wire of light along its
//             TOP edge, and drops a soft cast onto the ground beneath it.
//   RECESSED  is darker than its ground, casts an inner shadow along its TOP
//             edge, and catches a wire of light along its BOTTOM.
//
// The first version of this sheet painted controls straight onto one flat
// fill. With no neighbour to be relative to, nothing could read as raised or
// recessed, and the sheet became a stack of coloured rectangles. Depth is a
// relationship between adjacent tones, not a property of one widget, so the
// containers come first and the controls come second.

mod comp {
    use egui::Color32;

    // Surfaces: a ladder, not a set of absolute values.
    //
    // An earlier pass of this cleared the critique's "near-miss palette copies"
    // by taking `proto/glass-mock/index.html`'s colours verbatim — including its
    // `#1A1714` card. That was the wrong kind of faithfulness: the mock's card
    // sits over a BRIGHT TILE MAP, and its absolute value is dark because of
    // what is behind it. On this sheet's near-black ground the same number
    // lands at 1.18:1 against the ground, i.e. invisible, which I confirmed by
    // screenshot and by arithmetic.
    //
    // So the inks are the reference's and the surfaces are a ladder. Every ink
    // below is checked against the card it sits on rather than trusted.
    pub const GROUND: Color32 = Color32::from_rgb(0x0E, 0x0C, 0x09);
    /// One visible step up from the ground. 12–14/255 per channel.
    pub const CARD: Color32 = Color32::from_rgb(0x2B, 0x25, 0x1C);
    /// Recessed beds, cut into a card. Two steps down from `CARD`.
    pub const BED: Color32 = Color32::from_rgb(0x0A, 0x08, 0x07);
    /// The raised state: a selected segment, a hovered secondary button.
    pub const LIFT: Color32 = Color32::from_rgb(0x3F, 0x38, 0x2B);

    /// Ink levels, all measured against `CARD` (not against the ground).
    /// 12.4:1, 5.7:1 and 4.8:1 — the two that carry text both clear the 4.5:1
    /// body bar, and the third is used only for tracked-caps labels that
    /// repeat the note underneath.
    pub const INK: Color32 = Color32::from_rgb(0xED, 0xE6, 0xDA);
    pub const INK_MID: Color32 = Color32::from_rgb(0xA7, 0x9D, 0x8E);
    pub const INK_LOW: Color32 = Color32::from_rgb(0x9A, 0x90, 0x81);

    /// The warm pale the light is made of.
    pub const WIRE: Color32 = INK;
    /// The warm dark the shadow is made of.
    pub const SHADE: Color32 = Color32::from_rgb(0x05, 0x04, 0x03);

    /// The accent, single-sourced from the app's token. See `super::ACCENT`.
    pub const ACCENT: Color32 = super::ACCENT;
    /// Ink that sits on the accent and clears 4.5:1 against it.
    pub const ON_ACCENT: Color32 = Color32::from_rgb(0x0A, 0x0F, 0x1F);

    /// One radius for every rect this sheet draws.
    ///
    /// There were six in the prototype before this: 6, 7 and 8 in this file
    /// and 8, 12 and `tokens::ISLAND_RADIUS` in the harness. The app already
    /// names this exact number — `tokens::CONTROL_RADIUS`, "uniform across the
    /// whole widget family so a control is recognisable by silhouette alone" —
    /// so this cites it instead of minting a fourth local value that happens
    /// to match.
    pub const BED_RADIUS: u8 = crate::tokens::CONTROL_RADIUS;

    /// Field height. The sheet's rhythm lives here, in one number.
    pub const FIELD_H: f32 = 42.0;
}

/// The sheet's ground, re-exported so the window hosting the sheet and the
/// cards inside it step up from the same number.
pub const COMP_GROUND: Color32 = comp::GROUND;

// ---------------------------------------------------------------------------
// The lighting primitives
// ---------------------------------------------------------------------------

/// Lighten a colour, for hover states. `amt` is in gamma space, which is
/// where the palette is defined, so no conversion is applied.
fn lift(c: Color32, amt: u8) -> Color32 {
    let mut c = c.to_opaque();
    c[0] = c.r().saturating_add(amt);
    c[1] = c.g().saturating_add(amt);
    c[2] = c.b().saturating_add(amt);
    c
}

/// A wire of light along the TOP edge of a raised surface, clipped to it.
///
/// The clip is the whole point. Without it the line runs straight across the
///
/// rounded corners and the surface reads as a sticker rather than a solid.
fn top_wire(painter: &Painter, rect: Rect, radius: f32, color: Color32) {
    let pid = painter.with_clip_rect(rect);
    // Stopped at 2.5 radii rather than at one. A wire that reaches the
    // corner marks the corner, and the light only falls on the flat of the
    // edge.
    pid.line_segment(
        [
            pos2(rect.left() + radius * 2.5, rect.top() + 0.5),
            pos2(rect.right() - radius * 2.5, rect.top() + 0.5),
        ],
        Stroke::new(1.0, color),
    );
}

/// The same wire along the BOTTOM edge, which is where a recessed surface
/// catches the light that a raised one catches at its top.
fn bottom_wire(painter: &Painter, rect: Rect, radius: f32, color: Color32) {
    let pid = painter.with_clip_rect(rect);
    pid.line_segment(
        [
            pos2(rect.left() + radius, rect.bottom() - 0.5),
            pos2(rect.right() - radius, rect.bottom() - 0.5),
        ],
        Stroke::new(1.0, color),
    );
}

/// An inner shadow: `depth` one-pixel bands of darkness just inside the top
/// edge, falling off quadratically.
///
/// This is the only thing that separates "cut into" from "painted on", and it
/// costs `depth` rects.
fn inset_top(painter: &Painter, rect: Rect, depth: usize, color: Color32) {
    let pid = painter.with_clip_rect(rect);
    for i in 0..depth {
        let t = i as f32 / depth.max(1) as f32;
        let a = (1.0 - t).powi(2);
        pid.rect_filled(
            Rect::from_min_size(pos2(rect.left(), rect.top() + i as f32), vec2(rect.width(), 1.0)),
            CornerRadius::ZERO,
            color.gamma_multiply(a * 0.6),
        );
    }
}

/// Fill a rect as a RECESSED bed: darker than what it sits in, shadowed at
/// its top, lit at its bottom, and edged all the way round.
fn paint_bed(ui: &egui::Ui, rect: Rect, radius: u8, fill: Color32) {
    let p = ui.painter();
    p.rect_filled(rect, CornerRadius::same(radius), fill);
    inset_top(p, rect, 4, egui::Color32::BLACK.gamma_multiply(0.65));
    bottom_wire(p, rect, radius as f32, comp::WIRE.gamma_multiply(0.045));
    p.rect_stroke(
        rect,
        CornerRadius::same(radius),
        Stroke::new(1.0, egui::Color32::BLACK.gamma_multiply(0.6)),
        egui::StrokeKind::Inside,
    );
}

/// Tracked caps: the sheet's heading treatment.
///
/// `extra_letter_spacing` lives on `TextFormat`, so a plain `ui.label` cannot
/// carry it. This is the same `LayoutJob` `chrome.rs::tracked_caps` builds.
fn tracked(text: &str, size: f32, color: Color32) -> egui::text::LayoutJob {
    let mut job = egui::text::LayoutJob::default();
    job.append(
        text,
        0.0,
        egui::TextFormat {
            font_id: inter(size),
            color,
            extra_letter_spacing: size * 0.09,
            ..Default::default()
        },
    );
    job
}

// ---------------------------------------------------------------------------
// The container
// ---------------------------------------------------------------------------

/// The two tiers of the sheet.
///
/// Every card is a raised surface, so every card carries the same light story.
/// What separates them is how far each sits off the ground: the one card that
/// carries the field is the peak of the page, and the ones after it are
/// quieter steps down. Five identical cards is a list, not a design.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Tier {
    /// The field. The one control the sheet is here to show.
    Primary,
    /// Everything that configures around the field.
    Supporting,
}

impl Tier {
    fn pad(self) -> f32 {
        match self {
            Tier::Primary => 24.0,
            Tier::Supporting => 16.0,
        }
    }

    fn radius(self) -> u8 {
        match self {
            Tier::Primary => tokens::ISLAND_RADIUS,
            Tier::Supporting => comp::BED_RADIUS,
        }
    }

    /// The wire of light along the top edge, as a fraction of the pale ink.
    fn wire(self) -> f32 {
        match self {
            Tier::Primary => 0.10,
            Tier::Supporting => 0.045,
        }
    }

    /// How far the card lifts off the ground beneath it.
    fn cast(self) -> egui::epaint::Shadow {
        match self {
            Tier::Primary => egui::epaint::Shadow {
                offset: [0, 6],
                blur: 14,
                spread: 1,
                color: comp::SHADE.gamma_multiply(0.95),
            },
            Tier::Supporting => egui::epaint::Shadow {
                offset: [0, 2],
                blur: 8,
                spread: 0,
                color: comp::SHADE.gamma_multiply(0.8),
            },
        }
    }

    /// The gap between the heading and the control it names.
    fn lead(self) -> f32 {
        match self {
            Tier::Primary => 18.0,
            Tier::Supporting => 12.0,
        }
    }

    /// Heading size. Two steps, so the tier is legible without a border.
    fn title_pt(self) -> f32 {
        match self {
            Tier::Primary => 12.0,
            Tier::Supporting => 10.5,
        }
    }
}

/// One group of the sheet.
///
/// The container is the point of this pass. Each group gets one, because a
/// control can only read as raised or recessed relative to a surface it sits
/// on, and on a single flat fill there is no surface for it to be relative to.
fn card(ui: &mut egui::Ui, tier: Tier, add: impl FnOnce(&mut egui::Ui)) {
    let r = tier.radius();
    let inner = egui::Frame::new()
        .fill(comp::CARD)
        .corner_radius(r)
        .inner_margin(egui::Margin::same(tier.pad() as i8))
        .stroke(egui::Stroke::new(1.0, comp::WIRE.gamma_multiply(0.06)))
        .shadow(tier.cast())
        .show(ui, add);
    // Painted after the frame because the frame paints its fill first and this
    // wire belongs on top of the fill. It sits exactly on the top edge, where
    // no content can reach it.
    top_wire(ui.painter(), inner.response.rect, f32::from(r), comp::WIRE.gamma_multiply(tier.wire()));
}

/// A card's heading. More space above the heading than below it, so the
/// heading sits with the thing it names rather than with the card edge.
fn card_head(ui: &mut egui::Ui, tier: Tier, title: &str, note: &str) {
    ui.label(tracked(title, tier.title_pt(), comp::INK_LOW));
    ui.label(
        egui::RichText::new(note)
            .size(11.0)
            .color(comp::INK_LOW),
    );
    ui.add_space(tier.lead());
}

// ---------------------------------------------------------------------------
// Controls
// ---------------------------------------------------------------------------

/// The three fills one control needs, so `with_visuals` has one call shape.
#[derive(Clone, Copy)]
struct Looks {
    idle: Color32,
    hover: Color32,
    active: Color32,
}

/// Run `body` with egui's widget visuals swapped out, then restore.
///
/// `Button::fill` documents itself as overriding every on-hover effect, so a
/// control that needs a hover state cannot ask the button for one: the fill
/// has to go into the style the button reads from. `button_style` fills the
/// frame from `weak_bg_fill`, which is why that is the field set here and
/// `bg_fill` is not. Scoped to a single call, and restored on the way out.
fn with_visuals<R>(ui: &mut egui::Ui, looks: Looks, body: impl FnOnce(&mut egui::Ui) -> R) -> R {
    let restore = ui.style().visuals.widgets.clone();
    let mut w = restore.clone();
    w.inactive.weak_bg_fill = looks.idle;
    w.hovered.weak_bg_fill = looks.hover;
    w.active.weak_bg_fill = looks.active;
    ui.style_mut().visuals.widgets = w;
    let out = body(ui);
    ui.style_mut().visuals.widgets = restore;
    out
}

/// A button in the two states the sheet needs: one that carries the action and
/// one that carries an alternative.
///
/// Both are raised, so both take the wire of light along the top edge. The
/// difference is that the primary is filled with the accent and the secondary
/// is a hairline over the card.
fn button(ui: &mut egui::Ui, label: &str, primary: bool) -> egui::Response {
    let looks = if primary {
        Looks {
            idle: comp::ACCENT,
            hover: lift(comp::ACCENT, 28),
            active: lift(comp::ACCENT, 10),
        }
    } else {
        Looks {
            idle: Color32::TRANSPARENT,
            hover: comp::WIRE.gamma_multiply(0.06),
            active: comp::WIRE.gamma_multiply(0.10),
        }
    };
    let saved = ui.spacing().button_padding;
    ui.spacing_mut().button_padding = egui::vec2(14.0, 9.0);
    let r = with_visuals(ui, looks, |ui| {
        ui.add(
            egui::Button::new(
                egui::RichText::new(label)
                    .size(tokens::TEXT_BODY_PT)
                    .color(if primary { comp::ON_ACCENT } else { comp::INK_MID }),
            )
            .corner_radius(comp::BED_RADIUS)
            .stroke(if primary {
                egui::Stroke::NONE
            } else {
                egui::Stroke::new(1.0, comp::WIRE.gamma_multiply(0.12))
            }),
        )
    });
    ui.spacing_mut().button_padding = saved;
    top_wire(
        ui.painter(),
        r.rect,
        7.0,
        comp::WIRE.gamma_multiply(if primary { 0.16 } else { 0.05 }),
    );
    r
}

/// A checkbox whose box this file draws.
///
/// egui's stock checkbox takes neither a fill nor a radius at the call site,
/// and the recessed box IS the treatment: dark fill, inner shadow at the top,
/// a wire of light along the bottom, and the accent with a tick when on.
fn checkbox(ui: &mut egui::Ui, label: &str, on: &mut bool) -> egui::Response {
    let row_h = 24.0;
    let (rect, resp) = ui.allocate_exact_size(
        vec2(ui.available_width().min(240.0), row_h),
        egui::Sense::click(),
    );
    let box_rect = Rect::from_min_size(
        rect.min + vec2(0.0, (row_h - 18.0) * 0.5),
        vec2(18.0, 18.0),
    );
    let p = ui.painter();
    let hovered = resp.hovered();

    if *on {
        // On is RAISED: accent fill, light along the top, tick cut out of it.
        p.rect_filled(box_rect, CornerRadius::same(comp::BED_RADIUS), comp::ACCENT);
        top_wire(p, box_rect, f32::from(comp::BED_RADIUS), comp::WIRE.gamma_multiply(0.22));
        let c = box_rect.center();
        let s = 4.5;
        p.line_segment(
            [pos2(c.x - s, c.y), pos2(c.x - s * 0.2, c.y + s * 0.85)],
            Stroke::new(2.0, comp::ON_ACCENT),
        );
        p.line_segment(
            [
                pos2(c.x - s * 0.2, c.y + s * 0.85),
                pos2(c.x + s * 1.15, c.y - s * 0.9),
            ],
            Stroke::new(2.0, comp::ON_ACCENT),
        );
    } else {
        // Off is RECESSED: a bed cut into the card.
        paint_bed(
            ui,
            box_rect,
            6,
            if hovered { comp::LIFT.gamma_multiply(0.5) } else { comp::BED },
        );
    }
    p.rect_stroke(
        box_rect,
        CornerRadius::same(comp::BED_RADIUS),
        Stroke::new(
            1.0,
            if hovered {
                comp::WIRE.gamma_multiply(0.18)
            } else {
                comp::WIRE.gamma_multiply(0.06)
            },
        ),
        egui::StrokeKind::Inside,
    );

    p.text(
        box_rect.right_top() + vec2(10.0, 3.0),
        egui::Align2::LEFT_TOP,
        label,
        inter(tokens::TEXT_BODY_PT),
        if hovered { comp::INK } else { comp::INK_MID },
    );
    if resp.clicked() {
        *on = !*on;
    }
    resp
}

// ---------------------------------------------------------------------------
// The sheet
// ---------------------------------------------------------------------------

/// The specimen page: one card per control, so each control is judgeable
/// against a surface rather than against the window background.
///
/// Every widget owns its state through the caller's `&mut`, and only the frame
/// around each one is drawn here. A widget whose state is passed in cannot be
/// hovered, focused or clicked the way an operator would, so the real widget
/// runs and this file only frames it.
pub fn component_sheet(
    ui: &mut egui::Ui,
    text: &mut String,
    selected: &mut usize,
    toggled: &mut [bool; 2],
    // Set true for exactly one frame to give the field focus, which is the
    // only way to capture the focus ring unattended.
    request_field_focus: &mut bool,
) {
    // The repo's own rhythm, so the specimen shows the console's real spacing
    // rather than egui's stock default.
    ui.spacing_mut().item_spacing = tokens::ITEM_SPACING;

    // Focus has to travel out of the field's card to the status line at the
    // bottom, and the closures are not allowed to borrow the same frame
    // twice, so it goes through a cell.
    let focused = std::cell::Cell::new(false);

    // auto_shrink off, so the scroll area claims the full height it was
    // given instead of collapsing to its content's size.
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
        ui.add_space(2.0);

        // --- Field ---------------------------------------------------------
        card(ui, Tier::Primary, |ui| {
            card_head(ui, Tier::Primary, "Field", "singleline, 42px, recessed bed, focus rim");

            let w = ui.available_width();
            let resp = ui.add_sized(
                vec2(w, comp::FIELD_H),
                egui::TextEdit::singleline(text)
                    .id(egui::Id::new("glass_field"))
                    .frame(egui::Frame::NONE)
                    .font(inter(tokens::TEXT_BODY_PT))
                    .text_color(comp::INK)
                    .margin(egui::Margin::symmetric(13, 11)),
            );
            // The hint needs its own colour, because `text_color` does not
            // reach the placeholder.
            if text.is_empty() {
                ui.painter().text(
                    resp.rect.min + vec2(13.0, 11.0),
                    egui::Align2::LEFT_TOP,
                    "Search settings",
                    inter(tokens::TEXT_BODY_PT),
                    comp::INK_LOW,
                );
            }
            if *request_field_focus {
                resp.request_focus();
                *request_field_focus = false;
            }
            paint_bed(ui, resp.rect, comp::BED_RADIUS, comp::BED);
            if resp.has_focus() {
                focused.set(true);
                // The one place the accent is allowed to glow: the surface
                // that owns input. Drawn OUTSIDE the bed so the rim reads as
                // a lit edge rather than as a border on the fill.
                ui.painter().rect_stroke(
                    resp.rect,
                    CornerRadius::same(comp::BED_RADIUS),
                    Stroke::new(2.0, comp::ACCENT),
                    egui::StrokeKind::Outside,
                );
            }
        });

        ui.add_space(16.0);

        // --- Segmented control ---------------------------------------------
        card(ui, Tier::Supporting, |ui| {
            card_head(ui, Tier::Supporting, "Segmented control", "one selected, raised out of a recessed track");

            let labels = ["Split", "Replace", "New tab"];
            let now = *selected;
            ui.horizontal(|ui| {
            let track = egui::Frame::new()
                .fill(comp::BED)
                .corner_radius(comp::BED_RADIUS)
                .inner_margin(egui::Margin::same(3))
                .stroke(egui::Stroke::new(1.0, egui::Color32::BLACK.gamma_multiply(0.6)))
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing = egui::vec2(3.0, 0.0);
                    for (i, l) in labels.iter().enumerate() {
                        let on = now == i;
                        let looks = Looks {
                            idle: if on { comp::LIFT } else { Color32::TRANSPARENT },
                            hover: if on {
                                lift(comp::LIFT, 8)
                            } else {
                                comp::WIRE.gamma_multiply(0.05)
                            },
                            active: comp::WIRE.gamma_multiply(0.09),
                        };
                        let r = with_visuals(ui, looks, |ui| {
                            ui.add(
                                egui::Button::new(
                                    egui::RichText::new(*l)
                                        .size(tokens::TEXT_BODY_PT - 1.0)
                                        .color(if on { comp::INK } else { comp::INK_MID }),
                                )
                                .corner_radius(comp::BED_RADIUS)
                                .stroke(egui::Stroke::NONE),
                            )
                        });
                        if on {
                            top_wire(ui.painter(), r.rect, f32::from(comp::BED_RADIUS), comp::WIRE.gamma_multiply(0.08));
                        }
                        if r.clicked() {
                            *selected = i;
                        }
                    }
                });
            inset_top(
                ui.painter(),
                track.response.rect,
                4,
                egui::Color32::BLACK.gamma_multiply(0.6),
            );
            bottom_wire(
                ui.painter(),
                track.response.rect,
                f32::from(comp::BED_RADIUS),
                comp::WIRE.gamma_multiply(0.04),
            );
            });
        });

        ui.add_space(16.0);

        // --- Toggles -------------------------------------------------------
        card(ui, Tier::Supporting, |ui| {
            card_head(ui, Tier::Supporting, "Toggles", "checkbox box drawn here, not by egui");

            for (i, name) in ["Wire", "Sound"].iter().enumerate() {
                checkbox(ui, name, &mut toggled[i]);
            }
        });

        ui.add_space(16.0);

        // --- Buttons -------------------------------------------------------
        card(ui, Tier::Supporting, |ui| {
            card_head(ui, Tier::Supporting, "Buttons", "raised; primary filled, secondary a hairline");

            let mut clicked = 0usize;
            ui.horizontal(|ui| {
                if button(ui, "Pause", false).clicked() {
                    clicked += 1;
                }
                ui.add_space(8.0);
                if button(ui, "Run", true).clicked() {
                    clicked += 2;
                }
            });
            if clicked > 0 {}
        });

        ui.add_space(16.0);

        // --- Tracked caps --------------------------------------------------
        card(ui, Tier::Supporting, |ui| {
            card_head(ui, Tier::Supporting, "Tracked caps", "0.09em, 11px, uppercase");
            ui.label(tracked(
                "APPEARANCE   MODEL   SOUND",
                11.0,
                comp::INK_LOW,
            ));
        });

        ui.add_space(18.0);

        // Status line, so the sheet says out loud which state each control is
        // in. Reading it off a screenshot is guesswork; reading it off a
        // number is not.
        ui.label(tracked(
            "STATE",
            10.0,
            comp::INK_LOW,
        ));
        ui.label(
            egui::RichText::new(format!(
                "focus={} · segment={} · toggles={},{}",
                focused.get(),
                COMP_LABELS[*selected],
                toggled[0] as u8,
                toggled[1] as u8
            ))
            .monospace()
            .size(11.0)
            .color(comp::INK_LOW),
        );
    });
}

/// The segment labels, named once so the status line cannot drift from the
/// control it reports.
const COMP_LABELS: [&str; 3] = ["Split", "Replace", "New tab"];
