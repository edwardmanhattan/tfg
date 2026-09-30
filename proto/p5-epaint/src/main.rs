//! Throwaway prototype: is a hard-cut / diagonal panel language viable in
//! pure egui + epaint, for ARCONS?
//!
//! One brief — the chrome of the Roster island — answered three ways, each
//! diverging on a different axis, so the picker says *which idea* is worth
//! porting rather than which colour you prefer.
//!
//! - **Notch** — axis: silhouette economy. One chamfer, nothing else
//!   changes. Layout, hit-testing and keyboard focus are untouched. The
//!   minimum viable cut.
//! - **Blade** — axis: angular language. A sheared parallelogram with an
//!   angular entrance. The Persona-5 candidate, and the one that costs you
//!   a vertex hit-test and a focus story.
//! - **Screentone** — axis: texture over shape. Today's rounded silhouette
//!   unchanged, a halftone fill and a glow edge. Tests whether the
//!   interesting part is the pattern or the geometry.
//!
//! Everything here is epaint. No wgpu pipeline, no shader, no custom
//! `Shape` variant — the question is how far primitives get.
//!
//! Keys: `1`-`3` / `←` `→` switch, `R` replays the entrance, `H` shows the
//! rect egui actually allocated. `--no-motion` is the reduced-motion path.
//!
//! Scratch surface, not production code. Every variant reduces to a
//! `Vec<Pos2>` plus a fill and a stroke, which is the port.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use eframe::egui;
use egui::emath::easing;
use egui::epaint::TextShape;
use egui::{
    Align, Align2, Color32, CornerRadius, FontId, Id, Layout, Mesh, Pos2, Rect, Sense, Shape,
    Stroke, StrokeKind, Ui, UiBuilder, Vec2, pos2, vec2,
};

// The real island module, compiled straight out of the tfg tree. tfg itself
// is not built here — it drags in the MapLibre C++ core, and AGENTS.md says
// not to compile it on the user's behalf — but `src/chrome.rs` has no tfg
// types, so pointing at it here type-checks the file that actually ships,
// against the same egui 0.36 / epaint 0.36 the app resolves.
#[path = "../../../src/chrome.rs"]
mod chrome;

// The real camera tween, same trick: `src/camera.rs` depends only on egui,
// so `cargo test` here runs its `#[cfg(test)]` tests — which is the only way
// to verify the interruption behaviour without building the MapLibre core.
#[path = "../../../src/camera.rs"]
mod camera;

// The GPU probe, same trick. `src/gpuprobe.rs` touches only wgpu (via
// egui-wgpu's re-export) and tokio, so this crate compiles it against the
// same wgpu 30 the app resolves — which is the only way to find out that
// `depth_slice` exists or that `create_texture` returns a bare `Texture`.
#[path = "../../../src/gpuprobe.rs"]
mod gpuprobe;

// ---------------------------------------------------------------------------
// Tokens, mirrored from DESIGN.md and apply_ops_theme (main.rs:12294).
//
// The variants differ on geometry, not palette — deliberately. The One
// Signal Rule (DESIGN.md:194) reserves Radar Cyan for live state, and
// spending it on decoration would cost the one accent that carries
// meaning. So the question this answers is "is the *shape* language worth
// having", with the palette held honest throughout.
// ---------------------------------------------------------------------------

const NIGHT: Color32 = Color32::from_rgb(0x0F, 0x17, 0x2A);
const SLATE: Color32 = Color32::from_rgb(0x1E, 0x29, 0x3B);
const WELL: Color32 = Color32::from_rgb(0x02, 0x06, 0x17);
const INK: Color32 = Color32::from_rgb(0xE2, 0xE8, 0xF0);
const MUTED: Color32 = Color32::from_rgb(0x8C, 0x8C, 0x8C);
const LINE: Color32 = Color32::from_rgb(0x33, 0x41, 0x55);
const SIGNAL: Color32 = Color32::from_rgb(0x22, 0xD3, 0xCE);
const STALE: Color32 = Color32::from_rgb(0xF6, 0xC5, 0x6B);

/// Hard offset shadow, not a blur. DESIGN.md:272 makes depth tonal plus a
/// hairline, and a blur pass is the one item on the fill-rate list that
/// isn't affordable on the low end.
const SHADOW: Color32 = Color32::from_black_alpha(120);

/// Corner radius today's theme already uses (main.rs:12306).
const RADIUS: u8 = 8;

/// Maximum shear (Blade) and chamfer (Notch) at full entrance. Named
/// because the content rect has to clear them: a label placed at a fixed
/// inset from the panel's left edge will poke through a sheared silhouette
/// unless the inset accounts for the lean.
const LEAN: f32 = 22.0;
const CUT: f32 = 20.0;

/// Every entrance is one eased scalar in 0..=1. 150ms, cubic out —
/// inside the popover budget, and decelerating, because a panel arriving
/// should settle rather than accelerate away.
const ENTER_SECS: f32 = 0.15;

// ---------------------------------------------------------------------------
// Variants
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Variant {
    Notch,
    Blade,
    Screentone,
}

const VARIANTS: [(Variant, &str, &str); 3] = [
    (
        Variant::Notch,
        "Notch",
        "one chamfer · layout, hit-test and focus unchanged",
    ),
    (
        Variant::Blade,
        "Blade",
        "sheared silhouette · needs a vertex hit-test and a focus story",
    ),
    (
        Variant::Screentone,
        "Screentone",
        "silhouette untouched · halftone + edge glow only",
    ),
];

// ---------------------------------------------------------------------------
// Geometry
//
// The entire trick to a "diagonal widget" in egui is that it isn't one.
// It's a convex polygon, and the content still lives in the axis-aligned
// rect you allocated. So layout, scrolling, text shaping and accessibility
// keep working exactly as they do today, and only the paint changes.
// ---------------------------------------------------------------------------

/// `rect` with its top-right corner cut back by `cut` px. Five points,
/// convex, so `Shape::convex_polygon` tessellates it as two triangles.
fn chamfer_tr(rect: Rect, cut: f32) -> Vec<Pos2> {
    let cut = cut.clamp(0.0, (rect.width() * 0.5).min(rect.height()));
    vec![
        pos2(rect.left(), rect.top()),
        pos2(rect.right() - cut, rect.top()),
        pos2(rect.right(), rect.top() + cut),
        pos2(rect.right(), rect.bottom()),
        pos2(rect.left(), rect.bottom()),
    ]
}

/// `rect` as a parallelogram leaning right, contained in its own box: the
/// top edge is inset on the left, the bottom edge on the right.
fn sheared(rect: Rect, shear: f32) -> Vec<Pos2> {
    let shear = shear.clamp(0.0, rect.width() * 0.35);
    vec![
        pos2(rect.left() + shear, rect.top()),
        pos2(rect.right(), rect.top()),
        pos2(rect.right() - shear, rect.bottom()),
        pos2(rect.left(), rect.bottom()),
    ]
}

fn translated(pts: &[Pos2], d: Vec2) -> Vec<Pos2> {
    pts.iter().map(|p| *p + d).collect()
}

/// Even-odd point-in-polygon. The real app already has this test for map
/// zones (`src/overlay.rs` `hit_test`, over `convex_hull` at main.rs:855),
/// so closing the click gap on a sheared panel reuses it verbatim.
fn point_in_poly(pts: &[Pos2], p: Pos2) -> bool {
    let mut inside = false;
    for i in 0..pts.len() {
        let a = pts[i];
        let b = pts[(i + 1) % pts.len()];
        if (a.y > p.y) != (b.y > p.y) {
            let t = (p.y - a.y) / (b.y - a.y);
            if p.x < a.x + t * (b.x - a.x) {
                inside = !inside;
            }
        }
    }
    inside
}

// ---------------------------------------------------------------------------
// Paint helpers — all pure epaint
// ---------------------------------------------------------------------------

/// The same polygon, translated, in flat black, drawn before the fill.
/// The graphic-design shadow rather than a drop shadow: one extra mesh,
/// no blur, and it reads as an edge instead of a smudge.
fn hard_shadow(painter: &egui::Painter, pts: &[Pos2], offset: Vec2) {
    painter.add(Shape::convex_polygon(
        translated(pts, offset),
        SHADOW,
        Stroke::NONE,
    ));
}

/// Two-tone vertical wash across an arbitrary convex polygon: a triangle
/// fan from the centroid with per-vertex colours, which the tessellator
/// interpolates across each triangle. Works for a five- or six-point
/// outline, not just a quad — tfg's `paint_gradient` (main.rs:433) is the
/// two-triangle special case of this for a rect.
fn paint_wash(painter: &egui::Painter, pts: &[Pos2], top: Color32, bottom: Color32) {
    if pts.len() < 3 {
        return;
    }
    let n = pts.len() as f32;
    let cx = pts.iter().map(|p| p.x).sum::<f32>() / n;
    let cy = pts.iter().map(|p| p.y).sum::<f32>() / n;
    let ys = pts.iter().map(|p| p.y);
    let span = (ys.clone().fold(f32::MIN, f32::max) - ys.fold(f32::MAX, f32::min)).max(1.0);
    let at = |p: Pos2| top.lerp_to_gamma(bottom, ((p.y - cy) / span + 0.5).clamp(0.0, 1.0));

    let mut mesh = Mesh::default();
    mesh.colored_vertex(pos2(cx, cy), top.lerp_to_gamma(bottom, 0.5));
    for i in 0..pts.len() {
        let (a, b) = (pts[i], pts[(i + 1) % pts.len()]);
        mesh.colored_vertex(a, at(a));
        mesh.colored_vertex(b, at(b));
        let last = mesh.vertices.len() as u32 - 1;
        mesh.add_triangle(0, last - 1, last);
    }
    painter.add(mesh);
}

/// A dot tile in PHYSICAL pixels derived from a logical size, so a dot
/// keeps the same apparent size at any DPI.
///
/// This is the one place epaint is visibly behind a shader: procedural
/// screentone would get constant apparent dot size from `fwidth` for
/// free. Rebuilding the tile when the scale factor moves is the epaint
/// equivalent, and costs one upload per scale change.
fn dot_tile(dot_px: f32, period_px: f32) -> egui::ColorImage {
    let t = period_px.max(2.0).round() as usize;
    let mut img = egui::ColorImage::new([t, t], vec![Color32::TRANSPARENT; t * t]);
    let c = (t as f32 - 1.0) * 0.5;
    let r = (dot_px * 0.5).max(0.5);
    for y in 0..t {
        for x in 0..t {
            let (dx, dy) = (x as f32 - c, y as f32 - c);
            if dx * dx + dy * dy <= r * r {
                img.pixels[y * t + x] = Color32::WHITE;
            }
        }
    }
    img
}

/// Tile `tex` across `rect` at `step` logical px, fading vertically.
fn paint_tone(
    painter: &egui::Painter,
    tex: egui::TextureId,
    rect: Rect,
    step: f32,
    top_alpha: f32,
    bottom_alpha: f32,
) {
    let step = step.max(2.0);
    let cols = (rect.width() / step).ceil() as usize + 1;
    let rows = (rect.height() / step).ceil() as usize + 1;
    let mut mesh = Mesh::with_texture(tex);
    for r in 0..rows {
        for c in 0..cols {
            let x0 = rect.left() + c as f32 * step;
            let y0 = rect.top() + r as f32 * step;
            let x1 = (x0 + step).min(rect.right());
            let y1 = (y0 + step).min(rect.bottom());
            if x1 <= x0 || y1 <= y0 {
                continue;
            }
            let t = ((y0 - rect.top()) / rect.height().max(1.0)).clamp(0.0, 1.0);
            let col = Color32::from_white_alpha(
                ((top_alpha + (bottom_alpha - top_alpha) * t) * 255.0) as u8,
            );
            let base = mesh.vertices.len() as u32;
            for (pos, uv) in [
                (pos2(x0, y0), pos2(0.0, 0.0)),
                (pos2(x1, y0), pos2(1.0, 0.0)),
                (pos2(x1, y1), pos2(1.0, 1.0)),
                (pos2(x0, y1), pos2(0.0, 1.0)),
            ] {
                mesh.vertices.push(egui::epaint::Vertex { pos, uv, color: col });
            }
            mesh.add_triangle(base, base + 1, base + 2);
            mesh.add_triangle(base, base + 2, base + 3);
        }
    }
    painter.add(mesh);
}

/// A tracked, optionally rotated, single-line label.
///
/// `TextFormat` carries `extra_letter_spacing` and `TextShape` carries
/// `angle` — that pair is the whole typographic core of the look, and
/// neither needs custom code. P5's condensed, wide-tracked, slanted caps
/// fall straight out of them.
#[allow(clippy::too_many_arguments)] // a label is position + type + pose; bundling it hides the knobs
fn angled_label(
    ctx: &egui::Context,
    painter: &egui::Painter,
    at: Pos2,
    text: &str,
    size: f32,
    angle: f32,
    color: Color32,
    tracking: f32,
) {
    let mut job = egui::text::LayoutJob {
        wrap: one_line_wrap(),
        halign: Align::LEFT,
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
    let mut shape = TextShape::new(at, galley, color);
    shape.angle = angle;
    painter.add(Shape::Text(shape));
}

// ---------------------------------------------------------------------------
// Content — real hulls from assets/fleet.json, so the panel is judged at the
// density it will actually run at, not at three tidy lorem rows.
// ---------------------------------------------------------------------------

struct Row {
    name: &'static str,
    hull: &'static str,
    class: &'static str,
    stale: bool,
}

const ROSTER: &[Row] = &[
    Row { name: "KRI Ahmad Yani",     hull: "351", class: "frigate · ASW",        stale: false },
    Row { name: "KRI Yos Sudarso",    hull: "353", class: "frigate · ASW",        stale: false },
    Row { name: "KRI Cakra",          hull: "401", class: "SSK · 209/1300",       stale: false },
    Row { name: "KRI Ardadedali",     hull: "404", class: "SSK · DSME 209/1400",  stale: false },
    Row { name: "KRI Alugoro",        hull: "405", class: "SSK · DSME 209/1400",  stale: true  },
    Row { name: "KRI Fatahillah",     hull: "—",   class: "corvette",             stale: false },
    Row { name: "KRI Sungai Gerong",  hull: "—",   class: "patrol · 40m",         stale: true  },
    Row { name: "KRI Dewaruci",       hull: "—",   class: "auxiliary",            stale: false },
];

fn roster_ui(ui: &mut Ui, selected: &mut usize) {
    ui.spacing_mut().item_spacing.y = 0.0;
    let w = ui.available_width();
    for (i, row) in ROSTER.iter().enumerate() {
        let (rect, resp) = ui.allocate_exact_size(vec2(w, 34.0), Sense::click());
        let painter = ui.painter_at(rect);
        if i == *selected {
            painter.rect_filled(
                rect,
                CornerRadius::same(3),
                Color32::from_black_alpha(70),
            );
            // A 2px rule, not a fill. The selected row has to stay readable
            // at a glance, and DESIGN.md:283 puts the weight on the 2px
            // edge rather than on a colour change.
            painter.rect_filled(
                Rect::from_min_size(pos2(rect.left(), rect.center().y - 9.0), vec2(2.0, 18.0)),
                CornerRadius::same(1),
                SIGNAL,
            );
        }
        painter.text(
            pos2(rect.left() + 12.0, rect.top() + 7.0),
            Align2::LEFT_TOP,
            row.name,
            FontId::monospace(12.0),
            if row.stale { MUTED } else { INK },
        );
        painter.text(
            pos2(rect.right() - 12.0, rect.top() + 7.0),
            Align2::RIGHT_TOP,
            row.hull,
            FontId::monospace(11.0),
            MUTED,
        );
        painter.text(
            pos2(rect.left() + 12.0, rect.top() + 20.0),
            Align2::LEFT_TOP,
            row.class,
            FontId::proportional(10.5),
            MUTED.linear_multiply(0.8),
        );
        if row.stale {
            painter.circle_filled(pos2(rect.right() - 16.0, rect.bottom() - 8.0), 2.5, STALE);
        }
        if resp.clicked() {
            *selected = i;
        }
    }
}

// ---------------------------------------------------------------------------
// The stand-in map
//
// The panel has to be judged against a real figure-ground problem or the
// answer is meaningless: a hard-cut panel either separates from a busy
// night map or it doesn't. This stands in for MapLibre — grid, islands,
// trails, markers, one group zone — using the same overlay discipline tfg
// uses: trails under bodies under outlines.
// ---------------------------------------------------------------------------

fn map_backdrop(painter: &egui::Painter, r: Rect) {
    paint_wash(
        painter,
        &[
            r.left_top(),
            r.right_top(),
            r.right_bottom(),
            r.left_bottom(),
        ],
        WELL,
        NIGHT,
    );

    let grid = Stroke::new(1.0, LINE.linear_multiply(0.16));
    let mut x = r.left();
    while x < r.right() {
        painter.line_segment([pos2(x, r.top()), pos2(x, r.bottom())], grid);
        x += 64.0;
    }
    let mut y = r.top();
    while y < r.bottom() {
        painter.line_segment([pos2(r.left(), y), pos2(r.right(), y)], grid);
        y += 64.0;
    }

    for island in [
        vec![
            pos2(120.0, 140.0),
            pos2(210.0, 118.0),
            pos2(262.0, 176.0),
            pos2(238.0, 246.0),
            pos2(150.0, 232.0),
            pos2(104.0, 186.0),
        ],
        vec![
            pos2(300.0, 300.0),
            pos2(352.0, 286.0),
            pos2(378.0, 330.0),
            pos2(330.0, 356.0),
            pos2(296.0, 336.0),
        ],
        vec![
            pos2(520.0, 150.0),
            pos2(566.0, 132.0),
            pos2(596.0, 176.0),
            pos2(552.0, 204.0),
            pos2(516.0, 182.0),
        ],
    ] {
        painter.add(Shape::convex_polygon(
            island,
            SLATE.linear_multiply(0.55),
            Stroke::new(1.0, LINE.linear_multiply(0.4)),
        ));
    }

    // Layer 1 — trails.
    for (i, (fx, fy)) in [(0.18, 0.38), (0.42, 0.47), (0.63, 0.30), (0.81, 0.55), (0.30, 0.66)]
        .iter()
        .enumerate()
    {
        let (ox, oy) = (r.width() * fx, r.height() * fy);
        let stale = i == 3;
        let base = if stale { MUTED } else { SIGNAL };
        for k in 0..18 {
            let t = k as f32 / 18.0;
            painter.circle_filled(
                pos2(ox - t * 90.0, oy - t * 34.0),
                2.0,
                base.linear_multiply(0.35 * (1.0 - t)),
            );
        }
    }

    // Group zone — convex hull, the same primitive as main.rs:12035.
    painter.add(Shape::convex_polygon(
        vec![
            pos2(r.width() * 0.58, r.height() * 0.34),
            pos2(r.width() * 0.72, r.height() * 0.29),
            pos2(r.width() * 0.77, r.height() * 0.44),
            pos2(r.width() * 0.63, r.height() * 0.48),
        ],
        SIGNAL.linear_multiply(0.10),
        Stroke::new(2.0, SIGNAL.linear_multiply(0.5)),
    ));

    // Layers 2 and 3 — bodies, then the 2px outline DESIGN.md:283 asks for.
    for (i, (fx, fy)) in [(0.18, 0.38), (0.42, 0.47), (0.63, 0.30), (0.81, 0.55), (0.30, 0.66)]
        .iter()
        .enumerate()
    {
        let p = pos2(r.width() * fx, r.height() * fy);
        painter.circle_filled(p, 5.0, if i == 3 { MUTED } else { SIGNAL });
        painter.circle_stroke(p, 5.0, Stroke::new(2.0, INK.linear_multiply(0.9)));
    }
}

// ---------------------------------------------------------------------------
// The panel
// ---------------------------------------------------------------------------

/// Paints the chrome for `variant` and returns the axis-aligned rect the
/// content should live in.
///
/// The content rect is inset from the visual rect and stays
/// axis-aligned in *every* variant. That is the reason a cut panel costs
/// paint and not layout — the reason `Notch` and `Blade` are viable at all
/// rather than being a rewrite.
fn panel_chrome(
    _ctx: &egui::Context,
    painter: &egui::Painter,
    variant: Variant,
    rect: Rect,
    enter: f32,
    tone: Option<egui::TextureId>,
    show_hitbox: bool,
) -> Rect {
    let e = enter.clamp(0.0, 1.0);
    // Applied to the chrome as a whole, so the entrance reads as one
    // object arriving rather than five parts fading independently.
    let alpha = (e * 1.6).min(1.0);
    let a = |c: Color32| c.linear_multiply(alpha);
    let body = a(SLATE);
    let body_lo = a(SLATE.linear_multiply(0.62));
    let edge = Stroke::new(1.0, a(LINE.linear_multiply(1.5)));

    let cut = CUT * e;
    let lean = LEAN * e;

    // Shadow, body, treatment, edge — in that order, always.
    match variant {
        Variant::Screentone => {
            painter.rect_filled(
                Rect::from_min_size(rect.min + vec2(5.0, 6.0), rect.size()),
                CornerRadius::same(RADIUS),
                SHADOW,
            );
            painter.rect_filled(rect, CornerRadius::same(RADIUS), body);
            paint_wash(
                painter,
                &[
                    rect.left_top(),
                    rect.right_top(),
                    rect.right_bottom(),
                    rect.left_bottom(),
                ],
                body,
                body_lo,
            );
            if let Some(tex) = tone {
                // Heaviest in the header band, gone by the roster. The
                // test is whether the pattern survives at reading size —
                // and the answer should be "no, which is why the roster
                // area is clean".
                let head = Rect::from_min_max(
                    rect.left_top(),
                    pos2(rect.right(), rect.top() + 72.0),
                );
                paint_tone(painter, tex, head, 7.0, 0.42 * alpha, 0.0);
            }
            painter.rect_stroke(rect, CornerRadius::same(RADIUS), edge, StrokeKind::Inside);
        }
        v => {
            let pts = match v {
                Variant::Blade => sheared(rect, lean),
                _ => chamfer_tr(rect, cut),
            };
            hard_shadow(painter, &pts, vec2(5.0, 6.0));
            painter.add(Shape::convex_polygon(pts.clone(), body, edge));

            if v == Variant::Blade {
                paint_wash(painter, &pts, body, body_lo);
                // A solid leading edge on the lean, so the silhouette has
                // weight instead of dissolving into the map.
                painter.line_segment(
                    [pts[0], pts[3]],
                    Stroke::new(2.0, a(SIGNAL.linear_multiply(0.45))),
                );
            } else if e > 0.02 {
                // Brighter along the cut only: it's the one edge a viewer
                // reads as a deliberate cut rather than a clipping bug.
                painter.line_segment(
                    [
                        pos2(rect.right() - cut, rect.top()),
                        pos2(rect.right(), rect.top() + cut),
                    ],
                    Stroke::new(2.0, a(SIGNAL.linear_multiply(0.55))),
                );
            }

            // The honest bit. On hover over the panel, show the rect egui
            // actually allocated, so the gap between "what looks
            // clickable" and "what is clickable" is visible rather than
            // argued. The app's existing point-in-polygon test closes the
            // click gap; keyboard focus would not.
            if show_hitbox {
                let d = 5.0;
                for (p, q) in [
                    (rect.left_top(), pos2(rect.left() + d, rect.top())),
                    (pos2(rect.right() - d, rect.top()), rect.right_top()),
                    (rect.right_bottom(), pos2(rect.right() - d, rect.bottom())),
                    (pos2(rect.left(), rect.bottom() - d), rect.left_bottom()),
                ] {
                    painter.line_segment(
                        [p, q],
                        Stroke::new(2.0, Color32::from_rgb(0xFF, 0x5C, 0x00)),
                    );
                }
                painter.rect_stroke(
                    rect,
                    CornerRadius::ZERO,
                    Stroke::new(1.0, Color32::from_rgba_unmultiplied(0xFF, 0x5C, 0x00, 120)),
                    StrokeKind::Middle,
                );
            }
        }
    }

    // A sheared silhouette's top-left corner is inset by the full lean, so
    // its content inset has to clear that plus a margin — otherwise the
    // header pokes through the diagonal.
    let pad_x = if variant == Variant::Blade { LEAN + 12.0 } else { 16.0 };
    Rect::from_min_max(
        pos2(rect.left() + pad_x, rect.top() + 14.0 + 22.0 * e),
        pos2(rect.right() - 16.0, rect.bottom() - 14.0),
    )
}

// ---------------------------------------------------------------------------
// The picker
//
// PICKER.md calls this harness chrome and forbids restyling it, so the
// values below are the spec's values translated to epaint: a floating dark
// pill, bottom centre, not theme-aware, one active item, a highlight that
// slides 250ms cubic-out while the variant swap itself stays instant. The
// swap is a 100+/session action, so it gets no transition of its own —
// same rule as the original.
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq)]
enum Pick {
    None,
    Switch(usize),
    Replay,
}

fn picker(ctx: &egui::Context, current: usize) -> Pick {
    let mut pick = Pick::None;
    egui::Area::new(Id::new("picker"))
        .order(egui::Order::Foreground)
        .anchor(Align2::CENTER_BOTTOM, vec2(0.0, -24.0))
        .show(ctx, |ui| {
            // Pill background: rgba(10,10,10,0.82) over a 999px radius,
            // with the spec's inset hairline standing in for the blurred
            // backdrop (egui has no backdrop blur outside the fx seam).
            let mut bg: Vec<Rect> = Vec::new();
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;

                for (i, (_, label, _)) in VARIANTS.iter().enumerate() {
                    let galley = ctx.fonts_mut(|f| f.layout_job(single_line(label, 13.0, Color32::WHITE)));
                    let size = galley.size();
                    let (rect, resp) = ui.allocate_exact_size(
                        vec2(size.x + 24.0, 28.0),
                        Sense::click(),
                    );
                    let painter = ui.painter_at(rect);
                    bg.push(rect);

                    if i == current {
                        let t = ctx.animate_bool_with_time_and_easing(
                            Id::new(("picker.hl", i)),
                            true,
                            0.25,
                            easing::cubic_out,
                        );
                        painter.rect_filled(
                            rect.shrink((1.0 - t) * 4.0),
                            CornerRadius::same(255),
                            Color32::from_white_alpha(31),
                        );
                    }

                    // :active { transform: scale(0.97) }
                    let s = if resp.is_pointer_button_down_on() { 0.97 } else { 1.0 };
                    let c = rect.center();
                    let color = if i == current {
                        Color32::WHITE
                    } else if resp.hovered() {
                        Color32::from_white_alpha(217)
                    } else {
                        Color32::from_white_alpha(140)
                    };
                    let g = ctx.fonts_mut(|f| f.layout_job(single_line(label, 13.0, color)));
                    let srect = Rect::from_center_size(c, s * rect.size());
                    let p = srect.left_top() + vec2(12.0, (srect.height() - g.size().y) * 0.5);
                    painter.add(Shape::Text(TextShape::new(p, g, color)));

                    if resp.clicked() {
                        pick = if i == current { Pick::Replay } else { Pick::Switch(i) };
                    }
                }

                // Divider, then replay — present because every variant has
                // an entrance worth re-running.
                let (d, _) = ui.allocate_exact_size(vec2(9.0, 16.0), Sense::hover());
                ui.painter().rect_filled(
                    Rect::from_center_size(d.center(), vec2(1.0, 16.0)),
                    CornerRadius::ZERO,
                    Color32::from_white_alpha(31),
                );
                let (r, rresp) = ui.allocate_exact_size(vec2(28.0, 28.0), Sense::click());
                let rp = ui.painter_at(r);
                bg.push(r);
                rp.text(
                    r.center(),
                    Align2::CENTER_CENTER,
                    "↻",
                    FontId::proportional(15.0),
                    if rresp.hovered() { Color32::WHITE } else { Color32::from_white_alpha(140) },
                );
                if rresp.clicked() {
                    pick = Pick::Replay;
                }
            });

            // Pill drawn last so it sits under the labels; the shadow is
            // three stacked translucent rounded rects, standing in for the
            // spec's two blurred drop shadows.
            let pill = bg
                .iter()
                .fold(None::<Rect>, |acc, r| Some(acc.map_or(*r, |a| a.union(*r))))
                .map(|r| r.expand2(vec2(4.0, 4.0)));
            if let Some(pill) = pill {
                let p = ui.painter();
                for (grow, alpha) in [(14.0, 10u8), (7.0, 18), (2.0, 26)] {
                    p.rect_filled(
                        pill.expand2(vec2(grow, grow)),
                        CornerRadius::same(255),
                        Color32::from_black_alpha(alpha),
                    );
                }
                p.rect_filled(pill, CornerRadius::same(255), Color32::from_black_alpha(209));
                p.rect_stroke(
                    pill,
                    CornerRadius::same(255),
                    Stroke::new(1.0, Color32::from_white_alpha(20)),
                    StrokeKind::Inside,
                );
            }
        });
    pick
}

/// One line, no wrap, no tracking — the picker's own labels. A tracked or
/// rotated label needs the explicit job below; the picker's is plain.
fn single_line(text: &str, size: f32, color: Color32) -> egui::text::LayoutJob {
    let mut job = egui::text::LayoutJob {
        wrap: one_line_wrap(),
        halign: Align::LEFT,
        ..Default::default()
    };
    job.append(
        text,
        0.0,
        egui::TextFormat {
            font_id: FontId::proportional(size),
            color,
            extra_letter_spacing: 0.0,
            ..Default::default()
        },
    );
    job
}

fn one_line_wrap() -> egui::text::TextWrapping {
    egui::text::TextWrapping {
        max_width: f32::INFINITY,
        max_rows: 1,
        break_anywhere: false,
        overflow_character: None,
    }
}

// ---------------------------------------------------------------------------

struct Proto {
    variant: usize,
    selected: usize,
    replay: u64,
    tone: Option<egui::TextureId>,
    tone_px: f32,
    show_hitbox: bool,
    motion: f32,
    /// Render the real shipped island module instead of the prototype
    /// panel. `--island` exists so `src/chrome.rs` is verified by the same
    /// screenshot pass that judges the prototypes, not only by the
    /// type-check.
    island_mode: bool,
    island_pos: Pos2,
    island_open: bool,
}

impl eframe::App for Proto {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        // Pinned so logical == physical and the window lands at a known
        // pixel rect. The screentone DPI path is still live — it just
        // reads this 1.0 rather than the display's scale.
        ctx.set_pixels_per_point(1.0);
        let screen = ui.max_rect();
        let ppp = ctx.pixels_per_point();

        // Behaviour contract from PICKER.md, unchanged.
        if !ctx.egui_wants_keyboard_input() {
            let n = VARIANTS.len() as i32;
            let step = if ctx.input(|i| i.key_pressed(egui::Key::ArrowRight)) {
                1
            } else if ctx.input(|i| i.key_pressed(egui::Key::ArrowLeft)) {
                n - 1
            } else {
                0
            };
            if step > 0 {
                self.variant = ((self.variant as i32 + step) % n) as usize;
                self.replay += 1;
            }
            for (i, key) in [egui::Key::Num1, egui::Key::Num2, egui::Key::Num3]
                .iter()
                .enumerate()
            {
                if ctx.input(|inp| inp.key_pressed(*key)) && self.variant != i {
                    self.variant = i;
                    self.replay += 1;
                }
            }
            if ctx.input(|i| i.key_pressed(egui::Key::R)) {
                self.replay += 1;
            }
            if ctx.input(|i| i.key_pressed(egui::Key::H)) {
                self.show_hitbox = !self.show_hitbox;
            }
        }

        // Rebuilt only when the scale factor moves, so DPI is handled
        // without a shader — at the cost of one upload per scale change.
        let period = 7.0 * ppp;
        if self.tone.is_none() || (self.tone_px - period).abs() > 0.5 {
            self.tone = Some(
                ctx.load_texture("tone", dot_tile(3.0 * ppp, period), egui::TextureOptions::NEAREST)
                    .id(),
            );
            self.tone_px = period;
        }

        map_backdrop(&ui.painter_at(screen), screen);

        if self.island_mode {
            // The real module, exactly as tfg will call it.
            let spec = chrome::Island::new(Id::new("Roster"), "Roster", vec2(300.0, 420.0));
            let mut pos = self.island_pos;
            let mut open = self.island_open;
            let selected = self.selected;
            let mut selected = selected;
            chrome::island_scrolled(&ctx, &spec, &mut pos, &mut open, |ui| {
                ui.label(format!("{} ships — click a name to follow", ROSTER.len()));
                ui.separator();
                roster_ui(ui, &mut selected);
                ui.separator();
                ui.collapsing("Legend", |ui| {
                    ui.label("● per-ship color — live track (gray when stale, ~6s silence)");
                    ui.label("amber ring — old (backfilled) data, not live");
                    ui.label("yellow ring — camera follows this hull");
                    ui.label("blue ring — selected, open in Inspector");
                });
            });
            self.island_pos = pos;
            self.island_open = open;
            self.selected = selected;
            if !open {
                self.island_open = true;
            }
            picker(&ctx, self.variant);
            return;
        }

        let variant = VARIANTS[self.variant].0;
        // One eased scalar drives the whole entrance. Re-keying on
        // (variant, replay) is what makes R and a switch re-run it.
        let enter = ctx.animate_bool_with_time_and_easing(
            Id::new(("enter", self.variant, self.replay)),
            true,
            self.motion * ENTER_SECS,
            easing::cubic_out,
        );

        // Anchored left-of-centre so most of the map stays visible: the
        // panel is only judgeable against a real figure-ground problem.
        let panel = Rect::from_min_size(
            pos2(screen.left() + 72.0, screen.top() + 64.0),
            vec2(340.0, 430.0),
        );

        ui.scope_builder(
            UiBuilder::new()
                .max_rect(panel)
                .layout(Layout::top_down(Align::LEFT))
                .sense(Sense::hover()),
            |ui| {
                let painter = ui.painter_at(panel);
                let inner = panel_chrome(
                    &ctx, &painter, variant, panel, enter, self.tone, self.show_hitbox,
                );

                // Header. The rotated, tracked, short-caps label is the
                // part of the look that needs `TextShape.angle` plus
                // `extra_letter_spacing` rather than any custom geometry.
                angled_label(
                    &ctx,
                    &painter,
                    pos2(inner.left(), panel.top() + 14.0),
                    "ROSTER",
                    15.0,
                    if variant == Variant::Blade { -0.055 } else { 0.0 },
                    if variant == Variant::Notch { SIGNAL } else { INK },
                    2.4,
                );
                painter.text(
                    pos2(panel.right() - 16.0, panel.top() + 16.0),
                    Align2::RIGHT_TOP,
                    "8 / 125",
                    FontId::monospace(11.0),
                    MUTED,
                );
                painter.line_segment(
                    [
                        pos2(inner.left(), panel.top() + 46.0),
                        pos2(inner.right(), panel.top() + 46.0),
                    ],
                    Stroke::new(1.0, LINE.linear_multiply(enter.clamp(0.0, 1.0) * 1.5)),
                );

                let body = Rect::from_min_max(
                    pos2(inner.left(), panel.top() + 54.0),
                    inner.right_bottom(),
                );
                ui.scope_builder(
                    UiBuilder::new()
                        .max_rect(body)
                        .layout(Layout::top_down(Align::LEFT))
                        .sense(Sense::hover()),
                    |ui| roster_ui(ui, &mut self.selected),
                );
            },
        );

        // On hover, mark the parts of the allocated rect the painted
        // polygon does not cover: the dead zone on a sheared panel.
        let settled_poly = match variant {
            Variant::Screentone => None,
            Variant::Blade => Some(sheared(panel, LEAN)),
            Variant::Notch => Some(chamfer_tr(panel, CUT)),
        };
        if self.show_hitbox
            && let Some(poly) = &settled_poly
            && let Some(p) = ctx.input(|i| i.pointer.hover_pos())
            && panel.contains(p)
            && !point_in_poly(poly, p)
        {
            ui.painter().rect_filled(
                Rect::from_center_size(p, vec2(5.0, 5.0)),
                CornerRadius::ZERO,
                Color32::from_rgb(0xFF, 0x00, 0x00).linear_multiply(0.5),
            );
        }

        match picker(&ctx, self.variant) {
            Pick::Switch(i) => {
                self.variant = i;
                self.replay += 1;
            }
            Pick::Replay => self.replay += 1,
            Pick::None => {}
        }
    }
}

fn main() -> eframe::Result {
    let args: Vec<String> = std::env::args().collect();
    // `--gpu-probe` runs the real `src/gpuprobe.rs` and exits, so the
    // capability check can be exercised on a real machine rather than only
    // type-checked.
    if args.iter().any(|a| a == "--gpu-probe") {
        return match gpuprobe::probe() {
            Ok(()) => Ok(()),
            Err(e) => {
                eprintln!("probe failed: {e}");
                Ok(())
            }
        };
    }
    let motion = if args.iter().any(|a| a == "--no-motion") { 0.0 } else { 1.0 };
    // `--variant N` / `--hitbox` so a variant can be captured without a
    // key injection tool, which is the only way to screenshot this on a
    // machine that has no xdotool.
    let variant = args
        .iter()
        .position(|a| a == "--variant")
        .and_then(|i| args.get(i + 1))
        .and_then(|s| s.parse::<usize>().ok())
        .filter(|i| *i < VARIANTS.len())
        .unwrap_or(0);
    let hitbox = args.iter().any(|a| a == "--hitbox");
    let island_mode = args.iter().any(|a| a == "--island");

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([900.0, 720.0])
            .with_position([40.0, 40.0])
            .with_title("p5-epaint — 1-3 / arrows switch · R replay · H hitbox"),
        ..Default::default()
    };
    eframe::run_native(
        "p5-epaint",
        options,
        Box::new(move |_cc| {
            Ok(Box::new(Proto {
                variant,
                selected: 0,
                replay: 0,
                tone: None,
                tone_px: 0.0,
                show_hitbox: hitbox,
                motion,
                island_mode,
                island_pos: pos2(340.0, 90.0),
                island_open: true,
            }))
        }),
    )
}
