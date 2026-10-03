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
//
// `tokens` is declared at this crate's root rather than inside `chrome` so
// that `chrome.rs`'s `use crate::tokens;` resolves identically here and in
// tfg. That is the whole reason the import is spelled `crate::` and not
// `super::`: `super::tokens` would work in tfg and fail here, which is a
// failure that only shows up in this harness.
#[path = "../../../src/tokens.rs"]
mod tokens;
#[path = "../../../src/chrome.rs"]
mod chrome;

// The exercise lifecycle. A leaf with no tfg types, so its six tests run
// here in two seconds instead of behind a link against the MapLibre core.
// This is the only reason it is a module rather than a type in `main.rs`.
#[path = "../../../src/gamestate.rs"]
mod gamestate;

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

// The shader seam. Its WGSL is compiled by the real driver here rather than
// assumed valid, and the draw is checked for non-zero output.
#[path = "../../../src/fx/mod.rs"]
mod fx;

// The APP-6C symbology core. This is the module the harness exists for as
// much as the panel is: it has no egui and no sqlite in it, so it compiles
// here in about two seconds and its icon generator runs headless. That is
// why `Affiliation` and `BattleDimension` moved out of `store` and
// `map_render` into it — as long as a symbology type reached through
// `store`, this line would drag rusqlite's bundled amalgamation behind
// every vocabulary review.
#[path = "../../../src/symbology/mod.rs"]
mod symbology;

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
// Heat pass — the "tactical with heat" register, rendered rather than argued.
//
// Three sub-passes, each answering a different question the register cannot
// be judged without:
//
// - `Heat::Seam` — a lit inner rim on the island. The question is whether a
//   glow can stay on a panel edge without becoming the thing you look at.
// - `Heat::Vent` — a machined slot and notch detail on the title band. The
//   question is whether "built object" detail survives at reading density.
// - `Heat::Scanline` — a fine horizontal rule texture, the CRT texture of
//   the register. The question is the one DESIGN.md answered "no" before:
//   does it hold up over an information-dense body?
//
// `Heat::None` is the control. It renders the notch with today's paint, so
// every judgement below is against the shipped baseline rather than against
// a memory of it.
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Heat {
    None,
    Seam,
    Vent,
    Scanline,
}

const HEATS: [(Heat, &str, &str); 4] = [
    (Heat::None, "None", "today's paint · the control"),
    (Heat::Seam, "Seam", "lit inner rim · glow on an edge"),
    (Heat::Vent, "Vent", "machined slots · built-object detail"),
    (Heat::Scanline, "Scan", "rule texture · the CRT question"),
];

/// Inner rim brightness. Deliberately low: a rim that competes with the
/// panel's own title text has already failed, and the only way to know the
/// threshold is to render it.
const RIM: Color32 = Color32::from_rgb(0x22, 0xD3, 0xEE);

/// Depth of the inner rim band, px.
const RIM_INSET: f32 = 1.0;
const RIM_BAND: f32 = 2.0;

/// Machined slot geometry on the title band, px.
const VENT_W: f32 = 3.0;
const VENT_H: f32 = 9.0;
const VENT_GAP: f32 = 5.0;

/// Horizontal rule spacing for the scanline pass, px.
const SCAN_PITCH: f32 = 4.0;

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

/// The empty stage: near-black, no map, no markers.
///
/// An emissive edge reads completely differently over a lit map and over a
/// void, and the app has both — a panel over open water and a panel over
/// the well during a boot. Judging the heat on the lit map alone would hide
/// the case where the glow is the only thing carrying the silhouette.
fn dark_stage(painter: &egui::Painter, r: Rect) {
    painter.rect_filled(r, CornerRadius::ZERO, WELL);
}

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
fn panel_chrome_heat(
    _ctx: &egui::Context,
    painter: &egui::Painter,
    variant: Variant,
    rect: Rect,
    enter: f32,
    tone: Option<egui::TextureId>,
    show_hitbox: bool,
    heat: Heat,
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

    // The heat passes, painted on the settled silhouette. Order matters:
    // the scanlines are texture and go on the body, the seam is an inner
    // edge and goes over the fill, the vents are hardware and go last so
    // nothing paints over them.
    if heat != Heat::None && e > 0.6 {
        let settled = if variant == Variant::Blade {
            sheared(rect, LEAN * e)
        } else {
            chamfer_tr(rect, CUT * e)
        };

        if heat == Heat::Scanline {
            paint_scanlines(painter, rect, e);
        }

        if heat == Heat::Vent {
            paint_vents(painter, rect, e);
        }

        // Every heat pass also shows the seam's resting state, so the
        // neutral-vs-lit question is judged on one panel rather than
        // remembered across two.
        if heat != Heat::None {
            paint_seam(painter, &settled, e, heat == Heat::Seam);
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
// Heat passes.
//
// Each one is a question the register cannot be argued about, so each is
// painted and looked at. `e` is the entrance scalar so a pass arrives with
// the panel rather than popping onto a settled one.
// ---------------------------------------------------------------------------

/// A lit inner rim, drawn as the silhouette's own outline offset inward.
///
/// The offset is why this reads as a rim and not as a stroke: the band sits
/// between the outline and the body fill, so it is brightest exactly where
/// the panel meets the map.
///
/// `hot` decides the colour, and the choice is the whole finding. Rendering
/// every island's rim in Radar Cyan looked good in isolation and spent the
/// one accent that carries meaning on every panel at once, so an operator
/// could no longer read "this surface owns input" from the glow. At rest the
/// rim is a neutral cool grey, a shade above the hairline; cyan is reserved
/// for the island the pointer is on. That is the rule extension ADR-0014
/// anticipated, and rendering it is what showed the alternative is wrong.
fn paint_seam(painter: &egui::Painter, pts: &[Pos2], e: f32, hot: bool) {
    let a = (e - 0.6) / 0.4;
    let mut inner: Vec<Pos2> = Vec::with_capacity(pts.len());
    let c = centroid(pts);
    for p in pts {
        let toward = c - *p;
        inner.push(*p + toward.normalized() * RIM_INSET);
    }
    let tint = if hot { RIM.linear_multiply(0.5) } else { Color32::from_rgb(0x8C, 0x9B, 0xAE).linear_multiply(0.34) };
    painter.add(Shape::closed_line(
        inner
            .iter()
            .copied()
            .chain(std::iter::once(inner[0]))
            .collect::<Vec<_>>(),
        Stroke::new(RIM_BAND, tint.linear_multiply(a)),
    ));
}

fn centroid(pts: &[Pos2]) -> Pos2 {
    let sum: Vec2 = pts.iter().fold(Vec2::ZERO, |acc, p| acc + p.to_vec2());
    (sum / pts.len() as f32).to_pos2()
}

/// Machined slots along the bottom edge of the title band.
///
/// The test is whether "built object" detail survives at reading density,
/// which is a question about count before it is a question about shape. Ten
/// slots reads as ventilation; forty reads as a dotted line, and a dotted
/// line is not hardware.
fn paint_vents(painter: &egui::Painter, rect: Rect, e: f32) {
    let a = (e - 0.6) / 0.4;
    let y = rect.top() + 30.0 - 5.0;
    let mut x = rect.right() - 16.0 - 5.0 * (VENT_W + VENT_GAP);
    while x > rect.left() + 30.0 {
        let slot = Rect::from_center_size(pos2(x, y), vec2(VENT_W, VENT_H));
        painter.rect_filled(slot, CornerRadius::same(1), WELL.linear_multiply(0.9 * a));
        painter.rect_stroke(
            slot,
            CornerRadius::same(1),
            Stroke::new(1.0, LINE.linear_multiply(1.6 * a)),
            StrokeKind::Inside,
        );
        x -= VENT_W + VENT_GAP;
    }
}

/// A fine horizontal rule texture over the TITLE BAND only.
///
/// Rendered across the whole body it failed, and the reason is worth
/// recording because it is the same reason the vent pass failed: the rules
/// cut straight through the rows, and the smallest text on the panel (the
/// class line under each hull name) is the first thing to lose. Texture that
/// crosses text does not read as texture, it reads as interference.
///
/// Confined to the band it costs nothing, because the band holds one short
/// tracked label and a count.
fn paint_scanlines(painter: &egui::Painter, rect: Rect, e: f32) {
    let a = (e - 0.6) / 0.4;
    let band_bottom = rect.top() + 48.0;
    let mut y = rect.top() + SCAN_PITCH;
    while y < band_bottom {
        painter.line_segment(
            [pos2(rect.left(), y), pos2(rect.right() - CUT, y)],
            Stroke::new(1.0, Color32::from_white_alpha((17.0 * a) as u8)),
        );
        y += SCAN_PITCH;
    }
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
    picker_two(ctx, VARIANTS.iter().map(|v| v.1).collect(), current)
}

fn picker_two(ctx: &egui::Context, heats: Vec<&str>, current: usize) -> Pick {
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

                for (i, label) in heats.iter().enumerate() {
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
    heat: usize,
    heat_dark: bool,
    zone_mode: bool,
    zone_right: bool,
    modal_mode: bool,
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
    /// Paint a map-like bright ground behind everything.
    bright: bool,
    island_pos: Pos2,
    island_open: bool,
}

/// A stand-in for the real tile map: pale sea, white land, a grid, and a few
/// labels. Not a map — a background with the right LUMINANCE and contrast,
/// which is all a black overlay's behaviour depends on.
fn paint_bright_ground(ui: &mut egui::Ui, screen: egui::Rect) {
    let p = ui.painter_at(screen);
    let sea = egui::Color32::from_rgb(0xA8, 0xC8, 0xE8);
    let land = egui::Color32::from_rgb(0xF2, 0xF0, 0xE8);
    let ink = egui::Color32::from_rgb(0x40, 0x50, 0x60);

    p.rect_filled(screen, egui::CornerRadius::ZERO, sea);

    // Coastline-ish landmasses. Arbitrary blobs: the point is that large
    // areas sit near white, which is what a dim has to survive.
    for (fx, fy, fw, fh) in [
        (0.02f32, 0.02f32, 0.34f32, 0.30f32),
        (0.02, 0.02, 0.34, 0.30),
        (0.58, 0.10, 0.40, 0.46),
        (0.10, 0.62, 0.46, 0.34),
        (0.70, 0.70, 0.28, 0.26),
    ] {
        let r = egui::Rect::from_min_size(
            screen.min + egui::vec2(screen.width() * fx, screen.height() * fy),
            egui::vec2(screen.width() * fw, screen.height() * fh),
        );
        p.rect_filled(r, egui::CornerRadius::same(28), land);
        p.rect_stroke(
            r,
            egui::CornerRadius::same(28),
            egui::Stroke::new(1.5, egui::Color32::from_rgb(0xE8, 0xC8, 0x88)),
            egui::StrokeKind::Outside,
        );
    }

    // The geographic grid the real console draws over the map.
    let step = screen.width() / 6.0;
    let mut x = screen.left();
    while x < screen.right() {
        p.line_segment(
            [
                egui::pos2(x, screen.top()),
                egui::pos2(x, screen.bottom()),
            ],
            egui::Stroke::new(1.0, egui::Color32::from_rgb(0x88, 0xA0, 0xB8)),
        );
        x += step;
    }
    let mut y = screen.top();
    while y < screen.bottom() {
        p.line_segment(
            [
                egui::pos2(screen.left(), y),
                egui::pos2(screen.right(), y),
            ],
            egui::Stroke::new(1.0, egui::Color32::from_rgb(0x88, 0xA0, 0xB8)),
        );
        y += step;
    }

    // Labels, so there is fine bright text for the dim to compete with —
    // the thing a form most has to stay legible against.
    for (i, label) in ["Jakarta", "Surabaya", "Bandung", "Medan"].iter().enumerate() {
        p.text(
            egui::pos2(screen.left() + 24.0 + (i % 2) as f32 * 180.0, 40.0 + (i / 2) as f32 * 26.0),
            egui::Align2::LEFT_TOP,
            label,
            egui::FontId::proportional(14.0),
            ink,
        );
    }
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
            let hn = HEATS.len() as i32;
            let hstep = if ctx.input(|i| i.key_pressed(egui::Key::ArrowDown)) {
                1
            } else if ctx.input(|i| i.key_pressed(egui::Key::ArrowUp)) {
                hn - 1
            } else {
                0
            };
            if hstep > 0 {
                self.heat = ((self.heat as i32 + hstep) % hn) as usize;
                self.replay += 1;
            }
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
            for (i, key) in [egui::Key::Num7, egui::Key::Num8, egui::Key::Num9]
                .iter()
                .enumerate()
            {
                if ctx.input(|inp| inp.key_pressed(*key)) && self.heat != i {
                    self.heat = i;
                    self.replay += 1;
                }
            }
            if ctx.input(|i| i.key_pressed(egui::Key::R)) {
                self.replay += 1;
            }
            if ctx.input(|i| i.key_pressed(egui::Key::H)) {
                self.show_hitbox = !self.show_hitbox;
            }
            if ctx.input(|i| i.key_pressed(egui::Key::D)) {
                self.heat_dark = !self.heat_dark;
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

        // `--bright` paints a map-like ground: pale sea, white land, a grid
        // and some labels.
        //
        // This exists because the harness has NO background of its own — the
        // window is transparent, so every previous capture showed the desktop
        // wallpaper, which is nearly black. Judging the modal's backdrop dim
        // against a black wallpaper is judging it against nothing: the real
        // tile map is BRIGHT (white land, pale sea) and a black overlay lands
        // completely differently on it. The dim is the one modal decision
        // that cannot be judged without the right background, and the wrong
        // background is exactly what was there.
        if self.bright {
            paint_bright_ground(ui, screen);
        }

        if self.island_mode {
            // The real module, exactly as tfg will call it. `--zone` stacks
            // three islands down one docked column instead of showing a
            // single free-floating one, because the zone is the thing the
            // side-zone unit actually builds and a single island cannot
            // show whether the stack rhythm or the rim's one-at-a-time rule
            // holds.
            //
            // `--modal` puts the real modal over the real zone, which is the
            // only way to judge the two things that matter about a modal:
            // whether the dimmed map behind it still reads as a map, and
            // whether a form is legible on top of it.
            if self.modal_mode {
                let specs = vec![
                    (
                        chrome::Island::new(
                            Id::new("z.user"),
                            "Operator",
                            vec2(chrome::zone_width(), 112.0),
                        )
                        .with_trailing("Administrator"),
                        true,
                    ),
                    (
                        chrome::Island::new(
                            Id::new("z.players"),
                            "Players",
                            vec2(chrome::zone_width(), 320.0),
                        )
                        .with_trailing("6 SEATED"),
                        true,
                    ),
                ];
                let origins = chrome::zone_island_origins(chrome::Dock::Left, screen, &specs);
                let rects: Vec<egui::Rect> = specs
                    .iter()
                    .zip(&origins)
                    .map(|((s, _), p)| s.rect_at(*p))
                    .collect();
                let owner =
                    chrome::owning_island(&rects, ui.ctx().input(|i| i.pointer.hover_pos()));
                for (i, ((spec, _), mut pos)) in specs.iter().cloned().zip(origins).enumerate() {
                    let mut open = true;
                    chrome::island_owned(
                        ui.ctx(),
                        &spec,
                        &mut pos,
                        &mut open,
                        owner == Some(i),
                        |ui| {
                            ui.weak("a zone island, behind the dim");
                        },
                    );
                }

                let spec =
                    chrome::Modal::new(Id::new("players"), "Add players", vec2(640.0, 420.0));
                chrome::modal(&ctx, &spec, |ui| {
                    let mut search = String::new();
                    ui.label("Search the account directory");
                    ui.add(
                        egui::TextEdit::singleline(&mut search)
                            .desired_width(f32::INFINITY),
                    );
                    ui.separator();
                    for (name, role) in [
                        ("Super User", "Administrator"),
                        ("Rina Hartono", "Operator"),
                        ("Bagas Saputra", "Operator"),
                        ("Dewi Lestari", "Operator"),
                    ] {
                        ui.horizontal(|ui| {
                            ui.label(name);
                            ui.monospace(role);
                            if ui.small_button("seat").clicked() {}
                        });
                    }
                });
                return;
            }

            if self.zone_mode {
                let specs = vec![
                    (chrome::Island::new(Id::new("User"), "Operator", vec2(chrome::zone_width(), 96.0))
                        .with_trailing("OPERATOR"),
                     true),
                    (chrome::Island::new(Id::new("Connection"), "Connection", vec2(chrome::zone_width(), 132.0))
                        .with_trailing("LIVE"),
                     true),
                    (chrome::Island::new(Id::new("Roster"), "Roster", vec2(chrome::zone_width(), 300.0))
                        .with_trailing("8 / 125"),
                     true),
                ];
                let viewport = screen;
                let dock = if self.zone_right { chrome::Dock::Right } else { chrome::Dock::Left };
                let origins = chrome::zone_island_origins(dock, viewport, &specs);
                let selected = self.selected;
                let mut selected = selected;
                for ((spec, _), mut pos) in specs.iter().zip(origins) {
                    let mut open = true;
                    let title = spec.title.clone();
                    let trailing = spec.trailing.clone();
                    let z = chrome::zone_width();
                    if title == "Roster" {
                        chrome::island_scrolled(&ctx, spec, &mut pos, &mut open, |ui| {
                            roster_ui(ui, &mut selected);
                        });
                    } else {
                        chrome::island(&ctx, spec, &mut pos, &mut open, |ui| {
                            ui.spacing_mut().item_spacing.y = 6.0;
                            ui.label(format!("{} — z{:.0}", title, z));
                        });
                    }
                    let _ = trailing;
                }
                self.selected = selected;
                picker_two(&ctx, HEATS.iter().map(|h| h.1).collect(), self.heat);
                return;
            }

            let spec = chrome::Island::new(Id::new("Roster"), "Roster", vec2(300.0, 420.0))
                .with_trailing("8 / 125");
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
            picker_two(&ctx, HEATS.iter().map(|h| h.1).collect(), self.heat);
            return;
        }

        let variant = VARIANTS[self.variant].0;
        let heat = HEATS[self.heat].0;
        if self.heat_dark {
            dark_stage(&ui.painter_at(screen), screen);
        }
        // One eased scalar drives the whole entrance. Re-keying on
        // (variant, replay) is what makes R and a switch re-run it.
        let enter = ctx.animate_bool_with_time_and_easing(
            Id::new(("enter", self.variant, self.heat, self.replay)),
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
                let inner = panel_chrome_heat(
                    &ctx, &painter, variant, panel, enter, self.tone, self.show_hitbox, heat,
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
    // The symbology modes run before anything touches a window. They are
    // headless by construction: the module they drive has no egui in it,
    // which is the entire reason the vocabulary can be checked without a
    // graphics context and without the MapLibre build.
    if args.iter().any(|a| a == "--symbology-generate") {
        return match symbology_generate() {
            Ok(()) => Ok(()),
            Err(e) => {
                eprintln!("{e}");
                std::process::exit(1);
            }
        };
    }
    if args.iter().any(|a| a == "--symbology-check") {
        return match symbology_check() {
            Ok(()) => Ok(()),
            Err(e) => {
                eprintln!("{e}");
                std::process::exit(1);
            }
        };
    }
    // `--gpu-probe` runs the real `src/gpuprobe.rs` and exits, so the
    // capability check can be exercised on a real machine rather than only
    // type-checked.
    if args.iter().any(|a| a == "--fx-selftest") {
        return match fx_selftest() {
            Ok(()) => Ok(()),
            Err(e) => {
                eprintln!("fx selftest FAILED: {e}");
                std::process::exit(1);
            }
        };
    }
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
    let bright = args.iter().any(|a| a == "--bright");
    // `--heat N` selects a heat pass, `--heat-dark` renders it over the
    // near-black well instead of the map wash. Both exist because the only
    // honest way to judge an emissive edge is to see it on both grounds it
    // will ever sit on: over a lit map, and over an empty stage.
    let heat = args
        .iter()
        .position(|a| a == "--heat")
        .and_then(|i| args.get(i + 1))
        .and_then(|s| s.parse::<usize>().ok())
        .filter(|i| *i < HEATS.len())
        .unwrap_or(0);
    let heat_dark = args.iter().any(|a| a == "--heat-dark");
    // `--zone` stacks the side zone, `--zone-right` docks it right. Both
    // exist because the rim rule (ADR-0016) claims exactly one island lights
    // at a time, and that claim is only checkable with three on screen.
    let zone_mode = args.iter().any(|a| a == "--zone");
    let modal_mode = args.iter().any(|a| a == "--modal");
    let zone_right = args.iter().any(|a| a == "--zone-right");

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
                heat,
                heat_dark,
                zone_mode,
                zone_right,
                modal_mode,
                selected: 0,
                replay: 0,
                tone: None,
                tone_px: 0.0,
                show_hitbox: hitbox,
                motion,
                island_mode,
            bright,
                island_pos: pos2(340.0, 90.0),
                island_open: true,
            }))
        }),
    )
}


/// Rewrite `src/symbology/icons_generated.rs` from the two tables.
///
/// Separate from `--symbology-check` rather than a `--force` on it
/// because writing and asserting are different permissions, and a check
/// that can silently repair what it is checking is not a check.
fn symbology_generate() -> Result<(), String> {
    let out = symbology::generate::regenerate().map_err(|e| e.to_string())?;
    println!("wrote {}", out.display());
    Ok(())
}

/// Fail loudly if the checked-in geometry has drifted from the tables.
///
/// The generated file is committed, so this is the only thing standing
/// between a hand edit and a vocabulary whose readability invariants no
/// longer hold.
fn symbology_check() -> Result<(), String> {
    let generated = symbology::generate::generated_path().map_err(|e| e.to_string())?;
    match symbology::generate::check_drift(&generated).map_err(|e| e.to_string())? {
        None => {
            println!(
                "icons.tsv + milsymbol.tsv and {} agree",
                generated.display()
            );
            Ok(())
        }
        Some(diff) => Err(format!("icons_generated.rs is stale:\n\n{diff}")),
    }
}

/// Compile and draw the shipped halo on the real device, then read the
/// target back and assert it is not empty.
///
/// This is the only way to know the WGSL is valid: `cargo build` checks
/// Rust, not WGSL, and a bad shader only fails at pipeline creation at
/// runtime. It also proves the shader writes anything at all, rather than
/// compiling to a no-op.
///
/// The uniform is written here from the layout documented in `src/fx`,
/// independently of the module, so a drift between the two shows up as a
/// blank target rather than passing silently.
fn fx_selftest() -> Result<(), String> {
    use eframe::egui_wgpu::wgpu;

    const SIZE: u32 = 256;

    let rt = tokio::runtime::Builder::new_current_thread()
        .build()
        .map_err(|e| format!("runtime: {e}"))?;
    rt.block_on(async {
        let instance = wgpu::Instance::default();
        let mut adapters = instance.enumerate_adapters(wgpu::Backends::all()).await;
        if adapters.is_empty() {
            return Err("no adapter".to_string());
        }
        adapters.sort_by_key(|a| match a.get_info().device_type {
            wgpu::DeviceType::DiscreteGpu => 0,
            wgpu::DeviceType::IntegratedGpu => 1,
            _ => 2,
        });
        let adapter = adapters.swap_remove(0);
        let info = adapter.get_info();
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("fx selftest"),
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits::downlevel_defaults(),
                ..Default::default()
            })
            .await
            .map_err(|e| format!("device: {e}"))?;

        let fmt = wgpu::TextureFormat::Rgba8Unorm;
        let pipeline = fx::build_pipeline(&device, fmt).ok_or("build_pipeline returned None")?;
        println!("fx: pipeline built for {fmt:?} on {:?}", info.device_type);

        let tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("fx target"),
            size: wgpu::Extent3d { width: SIZE, height: SIZE, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: fmt,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = tex.create_view(&Default::default());

        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("fx uniform"),
            size: 32,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        // Centre of the target, radius HALO_R of 256px: centre_half = (0,0,half,half).
        let half = (fx::HALO_R / SIZE as f32) * 2.0;
        let mut bytes = [0u8; 32];
        for (i, f) in [0.0f32, 0.0, half, half, 0.133, 0.827, 0.933, 1.0].iter().enumerate() {
            bytes[i * 4..i * 4 + 4].copy_from_slice(&f.to_le_bytes());
        }
        queue.write_buffer(&uniform, 0, &bytes);

        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("fx bg"),
            layout: &pipeline.get_bind_group_layout(0),
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: uniform.as_entire_binding() }],
        });

        let mut enc = device.create_command_encoder(&Default::default());
        {
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("fx"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color { r: 0.0, g: 0.0, b: 0.0, a: 1.0 }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&pipeline);
            pass.set_bind_group(0, &bind, &[]);
            pass.draw(0..6, 0..1);
            drop(pass);
        }

        let row = SIZE * 4;
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("fx readback"),
            size: (row * SIZE) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        enc.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &tex,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(row),
                    rows_per_image: Some(SIZE),
                },
            },
            wgpu::Extent3d { width: SIZE, height: SIZE, depth_or_array_layers: 1 },
        );
        queue.submit([enc.finish()]);

        let slice = readback.slice(..);
        slice.map_async(wgpu::MapMode::Read, |_| {});
        device
            .poll(wgpu::PollType::Wait { submission_index: None, timeout: Some(std::time::Duration::from_secs(5)) })
            .map_err(|e| format!("poll: {e}"))?;
        let data = slice
            .get_mapped_range()
            .map_err(|e| format!("mapped range: {e:?}"))?;

        let centre = (SIZE as usize / 2 * row as usize) + SIZE as usize / 2 * 4;
        let edge_x = (SIZE as usize / 2 * row as usize) + (SIZE as usize * 3 / 4) * 4;
        let total = SIZE as usize * SIZE as usize * 4;
        let mut peak = 0u32;
        for i in (0..total).step_by(4) {
            let v = data[i] as u32 + data[i + 1] as u32 + data[i + 2] as u32;
            if v > peak {
                peak = v;
            }
        }
        let lit = data[centre] as u32 + data[centre + 1] as u32 + data[centre + 2] as u32;
        let outside = data[edge_x] as u32 + data[edge_x + 1] as u32 + data[edge_x + 2] as u32;
        drop(data);
        readback.unmap();

        println!("fx: peak channel sum {peak}, centre {lit}, off-radius {outside}");
        if peak == 0 {
            return Err("target is empty — the shader drew nothing".to_string());
        }
        if lit == 0 {
            return Err("nothing at the halo centre — uniform layout drifted".to_string());
        }
        if outside != 0 {
            return Err(format!("wrote {outside} outside the halo radius — falloff is wrong"));
        }
        println!("fx: OK — WGSL compiled by {:?}, drew, and falls off correctly", info.device_type);
        Ok::<(), String>(())
    })
}
