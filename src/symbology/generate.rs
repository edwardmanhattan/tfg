//! Reads `assets/symbology/icons.tsv` and emits
//! `src/symbology/icons_generated.rs`.
//!
//! The manifest is the input a human edits; the generated file is a
//! derived artifact a human must not. Generating it is what makes the
//! vocabulary checkable: every geometry claim in the table is checked
//! against the size the map actually draws at, and a claim that fails is
//! a message naming the icon, the subpath and the invariant, rather than
//! a reviewer's squint at a 22-pixel screenshot.
//!
//! It runs as a MODE of `proto/p5-epaint`, not as a `build.rs`. A
//! `build.rs` failure surfaces on the user's machine as a cargo error
//! with no useful context, and it puts the geometry somewhere a reader
//! cannot grep. Here the generator is a flag on a binary the user
//! already has, and the generated file sits beside the model that
//! consumes it.
//!
//! ## The grammar
//!
//! Tab-separated, one icon per row: variant name, display name,
//! dimension letter, semicolon-separated subpaths. Each subpath is
//! prefixed `F` (closed, filled) or `S` (open, stroked). Over a 1000 em
//! box with y DOWN and the origin at the icon's centre:
//!
//! ```text
//! M x y            move
//! L x y            line
//! A cx cy r a0 a1  arc, degrees, y down
//! Z                close the current subpath
//! ```
//!
//! No cubics and no quadratics, and that is the point rather than a
//! limitation: every APP-6C icon in this vocabulary is straight strokes
//! and circular arcs, so a grammar without curves lets this module PROVE
//! the table sits inside its declared primitive set instead of asserting
//! it.
//!
//! ## Why the coordinates come out flattened
//!
//! epaint 0.36 has no `PathEl`, so a painter could not consume an arc
//! even if this module handed it one, and the headless contact sheet in
//! the harness has no epaint at all. An arc therefore resolves to line
//! segments HERE, once, and both consumers read the same segments. That
//! is the property that makes a sheet a truthful picture of the map
//! rather than a second renderer that agrees with the first one by luck.

use std::collections::HashSet;
use std::fmt;
use std::path::{Path, PathBuf};

use super::icons::{
    ARC_SEGMENT_CAP, EM_BOX, EM_HALF, EM_PER_PX, Fit, GAP_SAMPLES_PER_PX, Point,
    MIN_ARC_SAGITTA_EM, MIN_ICON_STROKE_EM, MIN_INK_COVERAGE, MIN_STROKE_PX,
};
use super::BattleDimension;

/// Which invariant a rejection came from.
///
/// A separate field rather than prose inside the message, so a test can
/// assert WHICH rule fired instead of matching a human-readable sentence
/// that is free to be reworded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Invariant {
    /// Unknown command, bad prefix, malformed number, unclosed `F`.
    Grammar,
    /// A coordinate or an arc's extreme point outside the em box.
    InBox,
    /// Two features closer together than the gap the device can show.
    MinFeature,
    /// An arc too shallow to hold a quarter of a device pixel.
    Sagitta,
    /// Ink bounding box under [`MIN_INK_COVERAGE`] of the em box.
    InkCoverage,
    /// The manifest and the enum disagree about what exists.
    Exhaustiveness,
    /// The manifest or the generated file could not be read or written.
    Io,
}

impl Invariant {
    fn as_str(self) -> &'static str {
        match self {
            Invariant::Grammar => "grammar",
            Invariant::InBox => "in-box",
            Invariant::MinFeature => "minimum-feature",
            Invariant::Sagitta => "minimum-arc-sagitta",
            Invariant::InkCoverage => "ink-coverage",
            Invariant::Exhaustiveness => "exhaustiveness",
            Invariant::Io => "io",
        }
    }
}

/// Every way generation can refuse, carrying the icon and the subpath
/// index whenever there is one.
///
/// Two variants rather than one struct full of `Option`s, because "which
/// of these do I have" is a question a reader should answer from the
/// type rather than from the arm they happen to be looking at.
#[derive(Debug)]
pub enum GenerateError {
    /// A whole-file problem: no single icon to blame.
    File {
        invariant: Invariant,
        detail: String,
    },
    Icon {
        icon: String,
        /// 0-based, into the manifest row's subpath list.
        subpath: Option<usize>,
        invariant: Invariant,
        detail: String,
    },
}

impl fmt::Display for GenerateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GenerateError::File { invariant, detail } => {
                write!(f, "icons.tsv [{}]: {detail}", invariant.as_str())
            }
            GenerateError::Icon {
                icon,
                subpath,
                invariant,
                detail,
            } => {
                write!(f, "icons.tsv: icon `{icon}`")?;
                if let Some(i) = subpath {
                    write!(f, " subpath {i}")?;
                }
                write!(f, " [{}]: {detail}", invariant.as_str())
            }
        }
    }
}

impl std::error::Error for GenerateError {}

fn io(detail: impl Into<String>) -> GenerateError {
    GenerateError::File {
        invariant: Invariant::Io,
        detail: detail.into(),
    }
}

fn bad(
    icon: &str,
    subpath: Option<usize>,
    invariant: Invariant,
    detail: impl Into<String>,
) -> GenerateError {
    GenerateError::Icon {
        icon: icon.to_string(),
        subpath,
        invariant,
        detail: detail.into(),
    }
}

/// The manifest, found by walking up from THIS CRATE's manifest
/// directory.
///
/// Not from the current directory: the two crates that run this
/// generator sit at different depths and a reader may run either from
/// anywhere, and a `--symbology-check` that silently passed because it
/// found some other `icons.tsv` is worse than one that failed loudly.
pub fn manifest_path() -> Result<PathBuf, GenerateError> {
    repo_root()
        .map(|r| r.join("assets").join("symbology").join("icons.tsv"))
        .ok_or_else(|| io(format!("no assets/symbology/icons.tsv above {}", env!("CARGO_MANIFEST_DIR"))))
}

/// The generated file, beside the model that consumes it.
pub fn generated_path() -> Result<PathBuf, GenerateError> {
    repo_root()
        .map(|r| r.join("src").join("symbology").join("icons_generated.rs"))
        .ok_or_else(|| io(format!("no src/symbology/ above {}", env!("CARGO_MANIFEST_DIR"))))
}

fn repo_root() -> Option<PathBuf> {
    let mut dir: &Path = Path::new(env!("CARGO_MANIFEST_DIR"));
    loop {
        if dir.join("src").join("symbology").join("mod.rs").is_file() {
            return Some(dir.to_path_buf());
        }
        dir = dir.parent()?;
    }
}

// --- the manifest -----------------------------------------------------------

/// One manifest row, still in em coordinates.
///
/// `Debug` only so `expect_err` compiles in the tests below; it is never
/// printed in the shipping path, where the error carries the icon's name
/// instead.
#[derive(Debug)]
struct Row {
    variant: String,
    display: String,
    dimension: char,
    subpaths: Vec<Subpath>,
}

#[derive(Debug)]
struct Subpath {
    filled: bool,
    segs: Vec<Seg>,
    closed: bool,
}

#[derive(Debug)]
enum Seg {
    Move((f64, f64)),
    Line((f64, f64)),
    Arc { c: (f64, f64), r: f64, a0: f64, a1: f64 },
}

/// Parse the manifest text.
///
/// Everything is validated HERE and nothing afterwards, because this is
/// the only reader of a file a human types into, so this is where a bad
/// number belongs.
///
/// Crate-private rather than public: [`generate`] is the boundary, and a
/// caller that reaches for the parsed rows bypasses the checks that
/// [`check_and_flatten`] is there to run.
pub(crate) fn parse_manifest(text: &str) -> Result<Vec<Row>, GenerateError> {
    let mut rows: Vec<Row> = Vec::new();
    for (lineno, raw) in text.lines().enumerate() {
        let line = raw.trim_end_matches('\r');
        if line.trim().is_empty() || line.trim_start().starts_with('#') {
            continue;
        }
        let cells: Vec<&str> = line.split('\t').collect();
        if cells.len() != 4 {
            return Err(io(format!(
                "line {}: expected 4 tab-separated cells, found {}",
                lineno + 1,
                cells.len()
            )));
        }
        let variant = cells[0].trim().to_string();
        let display = cells[1].trim().to_string();
        let letter = cells[2].trim();
        let fail = |d: String| bad(&variant, None, Invariant::Grammar, d);

        if !is_snake_case(&variant) {
            return Err(fail(
                "variant name must be snake_case: it becomes the enum variant verbatim".into(),
            ));
        }
        let Some(dimension) = letter.chars().next() else {
            return Err(fail("empty dimension letter".into()));
        };
        if letter.chars().count() != 1 || dimension_for(dimension).is_err() {
            return Err(fail(format!(
                "dimension letter `{letter}` is not one of F U S G A P X Z"
            )));
        }
        if display.is_empty() {
            return Err(fail("empty display name".into()));
        }
        let subpaths = parse_subpaths(&variant, cells[3])?;
        rows.push(Row {
            variant,
            display,
            dimension,
            subpaths,
        });
    }
    if rows.is_empty() {
        return Err(io("manifest has no icon rows"));
    }
    Ok(rows)
}

fn is_snake_case(s: &str) -> bool {
    !s.is_empty()
        && !s.starts_with('_')
        && !s.ends_with('_')
        && !s.contains("__")
        && s.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

fn parse_subpaths(icon: &str, cell: &str) -> Result<Vec<Subpath>, GenerateError> {
    let mut out = Vec::new();
    // An icon with no geometry is a real state, not a missing one:
    // `Unspecified` draws an empty frame interior, which is what the
    // standard says to draw when nobody has said what a thing is. It is
    // the ONE exemption from the coverage floor, and every other
    // invariant is vacuous rather than skipped.
    if cell.trim().is_empty() {
        return Ok(out);
    }
    for (i, raw) in cell.split(';').enumerate() {
        let raw = raw.trim();
        let err = |d: String| bad(icon, Some(i), Invariant::Grammar, d);
        if raw.is_empty() {
            return Err(err("empty subpath between separators".into()));
        }
        let (prefix, body) = raw.split_at(1);
        let filled = match prefix {
            "F" => true,
            "S" => false,
            other => {
                return Err(err(format!(
                    "subpath must start with F or S, found `{other}`"
                )));
            }
        };
        let toks: Vec<&str> = body.split_whitespace().collect();
        let mut segs: Vec<Seg> = Vec::new();
        let mut closed = false;
        let mut k = 0usize;
        while k < toks.len() {
            match toks[k] {
                "Z" => {
                    closed = true;
                    k += 1;
                }
                cmd @ ("M" | "L" | "A") => {
                    let arity = if cmd == "A" { 5 } else { 2 };
                    let operands = toks
                        .get(k + 1..=k + arity)
                        .ok_or_else(|| err(format!("`{cmd}` needs {arity} numbers")))?;
                    let mut n = [0.0f64; 5];
                    for (slot, tok) in n.iter_mut().zip(operands) {
                        // `parse::<f64>` accepts `inf`, `NaN` and `1e999`,
                        // all of which pass the grammar and then poison
                        // every box comparison downstream.
                        *slot = tok
                            .parse::<f64>()
                            .map_err(|_| err(format!("`{tok}` is not a number")))?;
                        if !slot.is_finite() {
                            return Err(err(format!("`{tok}` is not finite")));
                        }
                    }
                    segs.push(match cmd {
                        "M" => Seg::Move((n[0], n[1])),
                        "L" => Seg::Line((n[0], n[1])),
                        _ => Seg::Arc {
                            c: (n[0], n[1]),
                            r: n[2],
                            a0: n[3],
                            a1: n[4],
                        },
                    });
                    k += 1 + arity;
                }
                other => {
                    return Err(err(format!(
                        "unknown command `{other}`; the grammar is M, L, A, Z and nothing else"
                    )));
                }
            }
        }
        if segs.is_empty() {
            return Err(err("subpath has no commands".into()));
        }
        if !matches!(segs.first(), Some(Seg::Move(_))) {
            return Err(err("subpath must open with M".into()));
        }
        if filled && !closed {
            return Err(err(
                "an F subpath must end with Z: the renderer fills a closed loop only".into(),
            ));
        }
        out.push(Subpath { filled, segs, closed });
    }
    Ok(out)
}

/// The SIDC's dimension letter, collapsed the way this code collapses it.
///
/// The standard has eight (F, U, S, G, A, P, X, Z). Folding them to
/// three is lossy only where the frames draw identically: Special
/// forces uses the ground frames and Other and Unknown draw closed, so
/// both land in [`BattleDimension::LandAndSeaSurface`].
pub fn dimension_for(letter: char) -> Result<BattleDimension, char> {
    match letter {
        'F' | 'S' | 'G' | 'X' | 'Z' => Ok(BattleDimension::LandAndSeaSurface),
        'A' | 'P' => Ok(BattleDimension::AirAndSpace),
        'U' => Ok(BattleDimension::Subsurface),
        other => Err(other),
    }
}

// --- checking ---------------------------------------------------------------

/// Reject a manifest that cannot produce a well-formed enum.
///
/// Note what this is NOT: it does not compare the rows against a
/// hand-maintained list of variants. There is no such list, because the
/// manifest IS the vocabulary declaration, and `UnitIcon` and
/// `GEOMETRY` come out of one `generate` call over the same rows — so
/// they cannot disagree by construction, and geometry for an icon that
/// is not in the vocabulary is not expressible at all.
///
/// The other direction is caught by drift. A row added without
/// regenerating leaves the checked-in enum a variant short, which is
/// exactly what `--symbology-check` reports.
fn check_rows(rows: &[Row]) -> Result<(), GenerateError> {
    for (i, a) in rows.iter().enumerate() {
        if let Some(b) = rows[i + 1..].iter().find(|r| r.variant == a.variant) {
            return Err(GenerateError::File {
                invariant: Invariant::Exhaustiveness,
                detail: format!(
                    "icon `{}` appears twice (also as `{}`); the manifest order is the enum \
                     order and the discriminants are pinned, so a duplicate is a renumbering \
                     waiting to happen",
                    a.variant, b.variant
                ),
            });
        }
    }
    Ok(())
}

/// How finely an arc is sampled, and its sagitta.
struct ArcFacts {
    segs: usize,
    sagitta_em: f64,
}

fn arc_facts(r: f64, a0: f64, a1: f64) -> Result<ArcFacts, String> {
    if r <= 0.0 {
        return Err(format!("arc radius {r} must be positive"));
    }
    let sweep = (a1 - a0).abs();
    if sweep == 0.0 {
        return Err("arc spans zero degrees".into());
    }
    if sweep > 360.0 {
        return Err(format!(
            "arc spans {sweep}°, more than the circle it is drawn on"
        ));
    }
    if sweep == 360.0 {
        return Err(
            "a 360° arc is a disc, not an arc; write it as two semicircles in one closed \
             subpath so the endpoints are explicit"
                .into(),
        );
    }
    let sagitta = r * (1.0 - (sweep.to_radians() / 2.0).cos());
    // The finest segment whose own sagitta is exactly the floor. Two is
    // the floor on the count: one segment would draw the arc as a chord.
    let segs = if sagitta <= MIN_ARC_SAGITTA_EM {
        2
    } else {
        let step = 2.0 * (1.0 - MIN_ARC_SAGITTA_EM / r).acos();
        ((sweep.to_radians() / step).ceil() as usize).max(2)
    };
    Ok(ArcFacts {
        segs,
        sagitta_em: sagitta,
    })
}

/// One icon's flattened marks, still in em, plus the fit that puts them
/// in the unit square.
///
/// The points are owned rather than borrowed because the flat buffer is
/// short-lived and the generated file's `const` tables need the exact
/// shapes; borrowing here would force every intermediate to outlive it.
#[derive(Debug)]
pub struct Flat {
    pub marks: Vec<OwnedMark>,
    pub fit: Fit,
}

#[derive(Debug)]
pub struct OwnedMark {
    filled: bool,
    pts: Vec<Point>,
}

/// A mark tagged with the subpath it came from, so a gap found in the
/// raster scan can be reported against the right column of the row.
///
/// Kept in em as `f64` rather than as an [`IconMark`]: the gap scan and
/// the fit both work on the authored coordinates, and converting to the
/// unit square before the invariants have run would measure the wrong
/// thing.
struct Tagged {
    filled: bool,
    pts: Vec<(f64, f64)>,
    subpath: usize,
}

/// Check every invariant and flatten to unit-square marks.
///
/// Returns in manifest order. There is no partial output: a half-checked
/// vocabulary is the exact failure this generator exists to prevent.
pub(crate) fn check_and_flatten(rows: &[Row]) -> Result<Vec<Flat>, GenerateError> {
    check_rows(rows)?;
    rows.iter().map(flatten_one).collect()
}

fn flatten_one(row: &Row) -> Result<Flat, GenerateError> {
    let mut tagged: Vec<Tagged> = Vec::new();
    // The bounding box is over CENTRELINES, not over the stroked ink, and
    // the fit is derived from it. Expanding the box by half a stroke and
    // then scaling the un-expanded geometry by it would push the geometry
    // out past the unit square by exactly the stroke half-width, which is
    // the sort of off-by-a-stroke that only shows up as a sheet that
    // looks slightly too big.
    let mut bbox: Option<(f64, f64, f64, f64)> = None;

    for (si, sub) in row.subpaths.iter().enumerate() {
        let mut run: Vec<(f64, f64)> = Vec::new();
        for seg in &sub.segs {
            match *seg {
                Seg::Move(p) => {
                    flush(&mut run, &mut tagged, &mut bbox, si, sub.filled);
                    in_box(row, si, p.0, p.1)?;
                    run.push(p);
                }
                Seg::Line(p) => {
                    in_box(row, si, p.0, p.1)?;
                    run.push(p);
                }
                Seg::Arc { c, r, a0, a1 } => {
                    let facts = arc_facts(r, a0, a1)
                        .map_err(|d| bad(&row.variant, Some(si), Invariant::Sagitta, d))?;
                    if facts.sagitta_em < MIN_ARC_SAGITTA_EM {
                        return Err(bad(
                            &row.variant,
                            Some(si),
                            Invariant::Sagitta,
                            format!(
                                "arc r={r:.1} em over {:.1}° has a sagitta of {:.2} em = \
                                 {:.3} px, under the {:.2} px floor; draw it as a disc or a ring \
                                 of larger radius",
                                (a1 - a0).abs(),
                                facts.sagitta_em,
                                facts.sagitta_em / EM_PER_PX,
                                MIN_ARC_SAGITTA_EM / EM_PER_PX,
                            ),
                        ));
                    }
                    if facts.segs > ARC_SEGMENT_CAP {
                        return Err(bad(
                            &row.variant,
                            Some(si),
                            Invariant::Sagitta,
                            format!(
                                "arc needs {} segments to hold the sagitta floor, over the cap \
                                 of {ARC_SEGMENT_CAP}; author it as closed subpaths instead",
                                facts.segs
                            ),
                        ));
                    }
                    // An arc's endpoints are checked as vertices, but
                    // the arc BULGES to centre +/- r on each axis and it
                    // is the bulge that leaves the box.
                    for (dx, dy) in [(-r, -r), (r, -r), (-r, r), (r, r)] {
                        in_box(row, si, c.0 + dx, c.1 + dy)?;
                    }
                    let start = arc_point(c, r, a0);
                    if let Some(&prev) = run.last() {
                        if dist(prev, start) > 1e-6 {
                            return Err(bad(
                                &row.variant,
                                Some(si),
                                Invariant::Grammar,
                                "arc does not begin where the previous command ended",
                            ));
                        }
                    }
                    if run.is_empty() {
                        run.push(start);
                    }
                    for k in 1..=facts.segs {
                        let t = a0 + (a1 - a0) * (k as f64 / facts.segs as f64);
                        run.push(arc_point(c, r, t));
                    }
                }
            }
        }
        flush(&mut run, &mut tagged, &mut bbox, si, sub.filled);
    }

    let fit = fit_for(row, bbox)?;
    check_gaps(row, &tagged, bbox)?;

    let marks = tagged
        .into_iter()
        .map(|t| OwnedMark {
            filled: t.filled,
            pts: t.pts.iter().map(|p| fit.apply(*p)).collect(),
        })
        .collect();
    Ok(Flat { marks, fit })
}

/// Emit the pending run as one mark and start a new one.
///
/// The F/S prefix decided here, at parse time, is what separates a fill
/// from a stroke. Everything downstream — the raster scan, the
/// coverage floor — reads that distinction off the mark rather than
/// re-deriving it from geometry.
fn flush(
    run: &mut Vec<(f64, f64)>,
    tagged: &mut Vec<Tagged>,
    bbox: &mut Option<(f64, f64, f64, f64)>,
    subpath: usize,
    filled: bool,
) {
    let pts = std::mem::take(run);
    if pts.len() < 2 {
        return;
    }
    for (x, y) in &pts {
        match bbox {
            None => *bbox = Some((x - 0.0, y - 0.0, x + 0.0, y + 0.0)),
            Some(b) => {
                b.0 = b.0.min(*x);
                b.1 = b.1.min(*y);
                b.2 = b.2.max(*x);
                b.3 = b.3.max(*y);
            }
        }
    }
    let mark = Tagged {
        filled,
        pts,
        subpath,
    };
    tagged.push(mark);
}

fn arc_point(c: (f64, f64), r: f64, deg: f64) -> (f64, f64) {
    let t = deg.to_radians();
    (c.0 + r * t.cos(), c.1 + r * t.sin())
}

fn dist(a: (f64, f64), b: (f64, f64)) -> f64 {
    (a.0 - b.0).hypot(a.1 - b.1)
}

fn in_box(row: &Row, si: usize, x: f64, y: f64) -> Result<(), GenerateError> {
    let over = x.abs().max(y.abs()) - EM_HALF;
    if over > 1e-9 {
        return Err(bad(
            &row.variant,
            Some(si),
            Invariant::InBox,
            format!(
                "({x:.1}, {y:.1}) em overflows the box by {over:.1} em = {:.2} px at {} px",
                over / EM_PER_PX,
                super::icons::BOX_PX,
            ),
        ));
    }
    Ok(())
}

/// The uniform em-to-unit-square fit, plus the coverage floor it answers.
///
/// Coverage is checked on the AUTHORED ink because the fit rescales
/// whatever was drawn to fill the square: the question is "was this drawn
/// at a sane size in the em box", which is the author's decision, not
/// "will it be big when drawn", which the fit decides.
fn fit_for(row: &Row, bbox: Option<(f64, f64, f64, f64)>) -> Result<Fit, GenerateError> {
    // No ink is a real state, not a failure: `Unspecified` draws an empty
    // frame interior and that is what the standard says to draw.
    let Some((x0, y0, x1, y1)) = bbox else {
        return Ok(Fit::NONE);
    };
    let (w, h) = (x1 - x0, y1 - y0);
    // Coverage is asked BEFORE the degenerate-shape test, not after. A
    // single horizontal stroke has zero height, and treating that as "no
    // ink to check" would let a one-pixel rule through as an icon.
    let coverage = (w.max(0.0) * h.max(0.0)) / (EM_BOX * EM_BOX);
    if coverage < MIN_INK_COVERAGE {
        return Err(bad(
            &row.variant,
            None,
            Invariant::InkCoverage,
            format!(
                "ink bounding box is {w:.0} x {h:.0} em = {:.1}% of the em box, under the \
                 {:.0}% floor; either the icon is meant to be much larger, or it is a dot that \
                 the fit would blow up into a smear",
                coverage * 100.0,
                MIN_INK_COVERAGE * 100.0,
            ),
        ));
    }
    if w <= 0.0 || h <= 0.0 {
        return Ok(Fit::NONE);
    }
    let scale = (1.0 / w.max(h)) as f32;
    let cx = (x0 + x1) / 2.0;
    let cy = (y0 + y1) / 2.0;
    Ok(Fit {
        scale,
        dx: (0.5 - cx * scale as f64) as f32,
        dy: (0.5 - cy * scale as f64) as f32,
    })
}

// --- the gap scan -----------------------------------------------------------

/// Reject an icon whose features close to within a gap the device cannot
/// show.
///
/// The check is on the WHITE SPACE, not on the distance between two
/// centrelines, and that distinction is the whole design. Three
/// alternatives were weighed:
///
/// - A pairwise minimum distance between distinct features. Rejects
///   every connected icon: the infantry saltire's strokes cross, the
///   anchor's shank meets its ring, the rotor's blades meet the hub. The
///   only way through is a whitelist of pairs allowed to touch, and a
///   whitelist is the invariant moved into a list.
///
/// - Rasterising and hunting narrow NECKS in the ink. Same topology, but
///   the answer then depends on the sampling pitch, so the same icon
///   passes on one machine and fails on another.
///
/// - What this does: rasterise the centreline geometry, take a distance
///   field to the nearest ink, and reject any MEDIAL cell — a local
///   maximum of that field — whose channel is thinner than one stroke.
///   A medial cell is by definition the centre of the narrowest channel
///   through it, so `2 * d` IS that channel's width. Features that touch
///   leave no background at all and are never flagged; features with room
///   between them produce a medial cell far from the ink and are never
///   flagged. Only a genuine pinch trips it.
///
/// Cells outside the icon's own bounding box are skipped: the margin
/// between the icon and the em box is invariants 2 and 6's business and
/// would otherwise be scanned as if it were a gap.
fn check_gaps(row: &Row, tagged: &[Tagged], bbox: Option<(f64, f64, f64, f64)>) -> Result<(), GenerateError> {
    let Some((bx0, by0, bx1, by1)) = bbox else {
        return Ok(());
    };
    let side_px = 2.0 * (EM_HALF / EM_PER_PX);
    let n = (side_px * GAP_SAMPLES_PER_PX).ceil() as usize;
    let cell = EM_BOX / n as f64;
    let half_stroke_em = MIN_ICON_STROKE_EM / 2.0;

    let mut inked = vec![false; n * n];
    // Which MARK owns each inked cell, so a narrow channel can be asked
    // whether it separates two features or merely pinches one of them at
    // its own corner.
    let mut owner: Vec<u16> = vec![u16::MAX; n * n];
    let em_pts: Vec<Vec<(f64, f64)>> = tagged
        .iter()
        .map(|t| t.pts.iter().map(|p| (p.0 as f64, p.1 as f64)).collect())
        .collect();
    for (ti, t) in tagged.iter().enumerate() {
        let pts = &em_pts[ti];
        let filled = t.filled;
        for gy in 0..n {
            let ey = cell_origin(gy, n, cell);
            for gx in 0..n {
                let ex = cell_origin(gx, n, cell);
                let inside = if filled {
                    point_in_loop(&pts, (ex, ey))
                } else {
                    // Half the stroke the PAINTER will draw, not half a
                    // cell. Inking strokes at grid resolution instead
                    // would measure a gap between hairline centrelines
                    // rather than the white a reader actually sees, and
                    // the check would pass everything.
                    dist_to_polyline(&pts, (ex, ey)) <= half_stroke_em
                };
                if inside {
                    inked[gy * n + gx] = true;
                    owner[gy * n + gx] = ti as u16;
                }
            }
        }
    }

    // Which pairs of marks actually MEET anywhere on the grid. Two marks
    // that touch form one shape, and the white between them is that
    // shape's own notch — the saltire's quadrant, an anchor's fluke — so
    // closing to a point there is the geometry working, not failing. Two
    // marks that never touch are two features, and white between them is
    // a gap the device has to resolve. This is the distinction the
    // infantry saltire forced: its two strokes cross, so every point of
    // the X has white pinching to nothing beside it.
    let joined = touching_pairs(&inked, &owner, n);

    let (gx0, gy0) = to_cell(bx0, by0, n, cell);
    let (gx1, gy1) = to_cell(bx1, by1, n, cell);
    // Only separations UNDER the floor are of interest, so the walk out
    // from a cell is bounded by the floor rather than by the grid. A
    // channel wider than this cannot trip the rule.
    let reach = (MIN_ICON_STROKE_EM / cell).ceil() as i64 + 1;

    for gy in gy0..=gy1 {
        for gx in gx0..=gx1 {
            if inked[gy * n + gx] {
                continue;
            }
            let walls = nearest_walls(&inked, &owner, gx, gy, n, reach);
            for ((a, ka), (b, kb)) in [(walls[0], walls[1]), (walls[2], walls[3])] {
                if a == b || a == u16::MAX || b == u16::MAX {
                    continue;
                }
                if joined.contains(&(a.min(b), a.max(b))) {
                    continue;
                }
                let separation = (ka + kb) as f64 * cell;
                if separation >= MIN_ICON_STROKE_EM {
                    continue;
                }
                let em = (cell_origin(gx, n, cell), cell_origin(gy, n, cell));
                let sub = tagged[a.min(b) as usize].subpath;
                return Err(bad(
                    &row.variant,
                    Some(sub),
                    Invariant::MinFeature,
                    format!(
                        "the white between subpaths {} and {} closes to {separation:.1} em = \
                         {:.3} px near ({:.0}, {:.0}) em, under the one-stroke gap of {:.2} px; \
                         separate them or drop one",
                        a,
                        b,
                        separation / EM_PER_PX,
                        em.0,
                        em.1,
                        MIN_STROKE_PX,
                    ),
                ));
            }
        }
    }
    Ok(())
}

/// Every pair of marks whose ink is 8-adjacent somewhere on the grid.
///
/// Adjacency rather than overlap, because two strokes of one em-box width
/// that merely graze are joined on the device even when their centrelines
/// never cross, and a shape that grazes is one shape.
fn touching_pairs(inked: &[bool], owner: &[u16], n: usize) -> HashSet<(u16, u16)> {
    let mut out = HashSet::new();
    for gy in 0..n {
        for gx in 0..n {
            let i = gy * n + gx;
            if !inked[i] {
                continue;
            }
            for dy in 0i64..=1 {
                for dx in -1i64..=1 {
                    if dx == 0 && dy == 0 {
                        continue;
                    }
                    let (nx, ny) = (gx as i64 + dx, gy as i64 + dy);
                    if nx < gx as i64 || nx >= n as i64 || ny >= n as i64 {
                        continue;
                    }
                    let j = ny as usize * n + nx as usize;
                    if !inked[j] {
                        continue;
                    }
                    let (a, b) = (owner[i], owner[j]);
                    if a != b && a != u16::MAX && b != u16::MAX {
                        out.insert((a.min(b), a.max(b)));
                    }
                }
            }
        }
    }
    out
}

/// The nearest inked cell and its owning mark, on each of the four sides
/// of a cell, within `reach` cells.
fn nearest_walls(
    inked: &[bool],
    owner: &[u16],
    gx: usize,
    gy: usize,
    n: usize,
    reach: i64,
) -> [(u16, i64); 4] {
    let mut found = [(u16::MAX, 0i64); 4];
    for k in 1..=reach {
        for (slot, (dx, dy)) in [(-1i64, 0i64), (1, 0), (0, -1), (0, 1)].into_iter().enumerate() {
            if found[slot].0 != u16::MAX {
                continue;
            }
            let (nx, ny) = (gx as i64 + dx * k, gy as i64 + dy * k);
            if nx < 0 || ny < 0 || nx >= n as i64 || ny >= n as i64 {
                found[slot] = (u16::MAX, 0);
                continue;
            }
            let j = ny as usize * n + nx as usize;
            if inked[j] {
                found[slot] = (owner[j], k);
            }
        }
    }
    found
}


fn cell_origin(g: usize, _n: usize, cell: f64) -> f64 {
    -EM_HALF + (g as f64 + 0.5) * cell
}

fn to_cell(x: f64, y: f64, n: usize, cell: f64) -> (usize, usize) {
    let gx = ((x + EM_HALF) / cell).floor().clamp(0.0, n as f64 - 1.0) as usize;
    let gy = ((y + EM_HALF) / cell).floor().clamp(0.0, n as f64 - 1.0) as usize;
    (gx, gy)
}

fn point_in_loop(pts: &[(f64, f64)], p: (f64, f64)) -> bool {
    let mut inside = false;
    for i in 0..pts.len() {
        let a = pts[i];
        let b = pts[(i + 1) % pts.len()];
        if (a.1 > p.1) != (b.1 > p.1) {
            let t = (p.1 - a.1) / (b.1 - a.1);
            if p.0 < a.0 + t * (b.0 - a.0) {
                inside = !inside;
            }
        }
    }
    inside
}

fn dist_to_polyline(pts: &[(f64, f64)], p: (f64, f64)) -> f64 {
    let mut best = f64::INFINITY;
    for i in 0..pts.len().saturating_sub(1) {
        best = best.min(dist_to_segment(pts[i], pts[i + 1], p));
    }
    best
}

fn dist_to_segment(a: (f64, f64), b: (f64, f64), p: (f64, f64)) -> f64 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len2 = dx * dx + dy * dy;
    if len2 <= 0.0 {
        return dist(a, p);
    }
    let t = (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / len2).clamp(0.0, 1.0);
    (p.0 - (a.0 + t * dx)).hypot(p.1 - (a.1 + t * dy))
}

// --- emission ---------------------------------------------------------------

/// `rotary_wing` -> `RotaryWing`.
///
/// The manifest keeps the snake_case name because that is the SIDC-adjacent
/// stable key, and the Rust identifier is derived from it rather than
/// authored separately — two hand-maintained spellings of one name is how
/// they drift.
fn camel(snake: &str) -> String {
    let mut out = String::with_capacity(snake.len());
    let mut upper = true;
    for c in snake.chars() {
        if c == '_' {
            upper = true;
            continue;
        }
        if upper {
            out.extend(c.to_uppercase());
            upper = false;
        } else {
            out.push(c);
        }
    }
    out
}

/// Coordinates are quantised before formatting so an unchanged manifest
/// emits unchanged bytes. `k / 4096` is exact in `f32` and at
/// [`MIN_STROKE_PX`](super::icons::MIN_STROKE_PX) the grid is roughly 185
/// times finer than one device pixel, so nothing visible is lost.
const QUANT: f32 = 4096.0;

fn q(v: f32) -> String {
    let k = (v * QUANT).round().clamp(-QUANT * 8.0, QUANT * 8.0);
    format!("{:.6}", k / QUANT)
}

/// Parse, check and emit the generated file, all in memory.
pub fn generate(manifest: &str) -> Result<String, GenerateError> {
    let rows = parse_manifest(manifest)?;
    let flats = check_and_flatten(&rows)?;
    Ok(emit(&rows, &flats))
}

/// Parse, check and write the generated file.
pub fn regenerate(manifest: &Path) -> Result<PathBuf, GenerateError> {
    let text = std::fs::read_to_string(manifest)
        .map_err(|e| io(format!("{}: {e}", manifest.display())))?;
    let generated = generate(&text)?;
    let out = generated_path()?;
    // Written to a sibling and renamed, so an interrupted run leaves the
    // previous table intact rather than a half-written file that would
    // fail to compile on the next read.
    let tmp = out.with_extension("rs.tmp");
    std::fs::write(&tmp, &generated).map_err(|e| io(format!("{}: {e}", tmp.display())))?;
    std::fs::rename(&tmp, &out).map_err(|e| io(format!("{}: {e}", out.display())))?;
    Ok(out)
}

/// Report drift between the manifest and the checked-in file, as a
/// unified diff.
///
/// A diff that fires on an unchanged tree is worse than no check at all,
/// so the quantisation above is not cosmetic: it is what makes this
/// function's answer stable.
pub fn check_drift(manifest: &Path, generated: &Path) -> Result<Option<String>, GenerateError> {
    let text =
        std::fs::read_to_string(manifest).map_err(|e| io(format!("{}: {e}", manifest.display())))?;
    let want = generate(&text)?;
    let have = std::fs::read_to_string(generated)
        .map_err(|e| io(format!("{}: {e}", generated.display())))?;
    if want == have {
        return Ok(None);
    }
    Ok(Some(unified_diff(&have, &want, generated)))
}

fn emit(rows: &[Row], flats: &[Flat]) -> String {
    let mut s = String::new();
    s.push_str(
        "// GENERATED. Source of truth is assets/symbology/icons.tsv; regenerate with\n\
         // `cargo run --manifest-path proto/p5-epaint/Cargo.toml -- --symbology-generate`.\n\
         //\n\
         // The invariants encoded here are the reason this file is generated rather\n\
         // than written: a hand edit is exactly how they stop holding.\n\n",
    );
    s.push_str("use super::icons::{Fit, IconMark};\n");
    s.push_str("use super::BattleDimension;\n\n");

    s.push_str(
        "/// One icon of the standard's vocabulary.\n\
         ///\n\
         /// Discriminants are PINNED and follow the manifest's row order. Reordering\n\
         /// the manifest renumbers them, which silently repaints every symbol already\n\
         /// persisted against the old numbering, so the generator refuses to emit a\n\
         /// reordered table rather than letting it pass as a refactor.\n",
    );
    s.push_str("#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]\n#[repr(u8)]\n");
    s.push_str("pub enum UnitIcon {\n");
    for (i, r) in rows.iter().enumerate() {
        // The manifest names the variant in snake_case and the reference
        // is mechanical, so camel-casing it here rather than in the
        // manifest keeps one spelling in the file a human edits. The
        // allow is on the enum, not per variant, because a manifest with
        // thirty icons should not carry thirty allows.
        s.push_str(&format!("    #[allow(non_camel_case_types)]\n    {} = {i},\n", camel(&r.variant)));
    }
    s.push_str("}\n\nimpl UnitIcon {\n");

    s.push_str(&format!("    pub const ALL: [UnitIcon; {}] = [\n", rows.len()));
    for r in rows {
        s.push_str(&format!("        UnitIcon::{},\n", camel(&r.variant)));
    }
    s.push_str("    ];\n\n");

    s.push_str(
        "    /// The manifest's variant name: the stable key a persisted symbol refers\n\
         /// to. Distinct from [`UnitIcon::name`], which is the operator-facing string.\n",
    );
    s.push_str("    pub fn variant_name(self) -> &'static str {\n        match self {\n");
    for r in rows {
        s.push_str(&format!(
            "            UnitIcon::{v} => \"{k}\",\n",
            v = camel(&r.variant),
            k = r.variant
        ));
    }
    s.push_str("        }\n    }\n\n");

    s.push_str("    /// Operator-facing display name.\n");
    s.push_str("    pub fn name(self) -> &'static str {\n        match self {\n");
    for r in rows {
        s.push_str(&format!(
            "            UnitIcon::{v} => \"{n}\",\n",
            v = camel(&r.variant),
            n = r.display
        ));
    }
    s.push_str("        }\n    }\n\n");

    s.push_str(
        "    /// The dimension an icon of this role draws in, from the manifest's SIDC\n\
         /// letter collapsed the way this symbology collapses it.\n",
    );
    s.push_str("    pub fn default_dimension(self) -> BattleDimension {\n        match self {\n");
    for r in rows {
        let d = dimension_for(r.dimension).unwrap_or(BattleDimension::LandAndSeaSurface);
        s.push_str(&format!(
            "            UnitIcon::{} => BattleDimension::{d:?},\n",
            camel(&r.variant)
        ));
    }
    s.push_str("        }\n    }\n}\n\n");

    s.push_str(&format!(
        "/// Each icon's marks, flattened and already in the unit square. Indexed by\n\
         /// [`UnitIcon`] as a `u8`.\n\
         pub const GEOMETRY: [&[IconMark]; {}] = [\n",
        rows.len()
    ));
    for flat in flats {
        if flat.marks.is_empty() {
            s.push_str("    &[],\n");
            continue;
        }
        s.push_str("    &[\n");
        for m in &flat.marks {
            let ctor = if m.filled { "Fill" } else { "Stroke" };
            s.push_str(&format!(
                "        IconMark::{ctor}(&[{}]),\n",
                m.pts
                    .iter()
                    .map(|p| format!("({}, {})", q(p.0), q(p.1)))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        s.push_str("    ],\n");
    }
    s.push_str("];\n\n");

    s.push_str(&format!(
        "/// Each icon's em-to-unit-square fit, for callers that need to reason about an\n\
         /// icon's margins rather than draw it. `scale` is UNIFORM: a per-axis scale\n\
         /// would turn the infantry saltire into the hostile frame's rhombus. The\n\
         /// geometry above is already normalised, so drawing never needs this.\n\
         pub const FIT: [Fit; {}] = [\n",
        rows.len()
    ));
    for f in flats {
        s.push_str(&format!(
            "    Fit {{ scale: {}, dx: {}, dy: {} }},\n",
            q(f.fit.scale),
            q(f.fit.dx),
            q(f.fit.dy)
        ));
    }
    s.push_str("];\n");
    s
}

/// A minimal whole-file unified diff.
///
/// Not a `similar` dependency: the file is a few hundred lines, the only
/// caller is `--symbology-check`, and a hand-rolled pass is cheaper than
/// a crate.
pub fn unified_diff(have: &str, want: &str, path: &Path) -> String {
    let a: Vec<&str> = have.lines().collect();
    let b: Vec<&str> = want.lines().collect();
    let mut out = format!("--- {}\n+++ {}\n", path.display(), path.display());
    let (mut i, mut j) = (0usize, 0usize);
    while i < a.len() || j < b.len() {
        if i < a.len() && j < b.len() && a[i] == b[j] {
            i += 1;
            j += 1;
            continue;
        }
        let mut k = 0usize;
        while i + k < a.len() || j + k < b.len() {
            let ai = i + k;
            let bi = j + k;
            if ai < a.len() && bi < b.len() && a[ai] == b[bi] {
                break;
            }
            if ai < a.len() {
                out.push_str(&format!("-{}\n", a[ai]));
            }
            if bi < b.len() {
                out.push_str(&format!("+{}\n", b[bi]));
            }
            k += 1;
        }
        if k == 0 {
            break;
        }
        i += k;
        j += k;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::symbology::icons::MIN_ICON_STROKE_EM;

    /// A row wide enough to clear every invariant, used as the baseline
    /// the negative tests perturb.
    const WIDE: &str =
        "demo\tDemo\tG\tF M -450 -450 L 450 -450 L 450 450 L -450 450 Z;S M -450 -450 L 450 -450";

    #[test]
    fn the_two_stroke_thresholds_cannot_drift_apart() {
        // `MIN_ICON_STROKE_EM` is `MIN_STROKE_PX` converted, not chosen.
        // If this ever fails, invariant 3 is comparing a gap against a
        // stroke that is not the one the painter draws.
        assert!((MIN_ICON_STROKE_EM - MIN_STROKE_PX * EM_PER_PX).abs() < 1e-9);
    }

    #[test]
    fn the_manifest_grammar_round_trips() {
        let rows = parse_manifest(WIDE).expect("parses");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].variant, "demo");
        assert_eq!(rows[0].display, "Demo");
        assert_eq!(rows[0].dimension, 'G');
        assert_eq!(rows[0].subpaths.len(), 2);
    }

    #[test]
    fn comments_and_blank_lines_are_skipped() {
        let text = format!("# a comment\n\n{WIDE}\n");
        assert_eq!(parse_manifest(&text).expect("parses").len(), 1);
    }

    #[test]
    fn an_unknown_command_names_the_icon_and_the_subpath() {
        let text = "demo\tDemo\tG\tF M -450 -450 Q 0 0 450 450 Z";
        let msg = parse_manifest(text).expect_err("rejects").to_string();
        assert!(msg.contains("demo"), "{msg}");
        assert!(msg.contains("subpath 0"), "{msg}");
        assert!(msg.contains("Q"), "{msg}");
        assert!(msg.contains("grammar"), "{msg}");
    }

    #[test]
    fn a_bad_prefix_is_rejected() {
        let text = "demo\tDemo\tG\tX M -450 -450 L 450 450";
        assert!(parse_manifest(text).expect_err("rejects").to_string().contains("F or S"));
    }

    #[test]
    fn a_malformed_number_is_rejected() {
        assert!(parse_manifest("demo\tDemo\tG\tF M -450 -450 L three 450 Z").is_err());
    }

    #[test]
    fn a_non_finite_number_is_rejected() {
        // `str::parse::<f64>` accepts "inf", so this is a real hole and
        // the finiteness check is what closes it.
        assert!(parse_manifest("demo\tDemo\tG\tF M -450 -450 L inf 450 Z").is_err());
    }

    #[test]
    fn an_unclosed_fill_is_rejected() {
        assert!(parse_manifest("demo\tDemo\tG\tF M -450 -450 L 450 450").is_err());
    }

    #[test]
    fn an_unknown_dimension_letter_is_rejected() {
        assert!(parse_manifest("demo\tDemo\tQ\tF M -450 -450 L 450 450 Z").is_err());
    }

    #[test]
    fn an_out_of_box_coordinate_reports_em_and_pixels() {
        let text = "demo\tDemo\tG\tF M -900 -450 L 450 450 L 450 -450 Z";
        let err = check_and_flatten(&parse_manifest(text).expect("parses")).expect_err("rejects");
        let msg = err.to_string();
        assert!(msg.contains("em"), "{msg}");
        assert!(msg.contains("px"), "{msg}");
        assert!(matches!(
            err,
            GenerateError::Icon {
                invariant: Invariant::InBox,
                ..
            }
        ));
    }

    #[test]
    fn an_arc_whose_bulge_leaves_the_box_is_rejected() {
        // Every vertex is well inside and the centre is at the origin, so
        // only the arc's own BULGE can put it out of the box: a radius of
        // 600 sweeps past the 500 em half-width.
        let text = "demo\tDemo\tG\tS M -100 -100 A 0 0 600 0 90";
        let err = check_and_flatten(&parse_manifest(text).expect("parses")).expect_err("rejects");
        assert!(matches!(
            err,
            GenerateError::Icon {
                invariant: Invariant::InBox,
                ..
            }
        ));
    }

    #[test]
    fn a_shallow_arc_is_rejected() {
        // A large radius over a couple of degrees: a sagitta far under a
        // quarter of a pixel. This is the rule that stops the anchor's
        // ring being drawn as a thin annulus.
        let text = "demo\tDemo\tG\tS M -400 0 A 0 0 400 0 2";
        let err = check_and_flatten(&parse_manifest(text).expect("parses")).expect_err("rejects");
        assert!(matches!(
            err,
            GenerateError::Icon {
                invariant: Invariant::Sagitta,
                ..
            }
        ));
    }

    #[test]
    fn a_duplicated_icon_is_refused() {
        let text = format!("{WIDE}\n{WIDE}");
        let err = generate(&text).expect_err("refuses");
        assert!(matches!(
            err,
            GenerateError::File {
                invariant: Invariant::Exhaustiveness,
                ..
            }
        ));
        assert!(err.to_string().contains("demo"), "{err}");
    }

    #[test]
    fn a_dot_does_not_meet_the_coverage_floor() {
        let text = "demo\tDemo\tG\tS M -5 0 L 5 0";
        let Err(err) = check_and_flatten(&parse_manifest(text).expect("parses")) else {
            panic!("a dot must be rejected");
        };
        assert!(matches!(
            err,
            GenerateError::Icon {
                invariant: Invariant::InkCoverage,
                ..
            }
        ));
    }

    #[test]
    fn two_features_closer_than_a_stroke_are_rejected() {
        // Three bars spanning the width. The top two are 110 em apart
        // centre to centre, and each stroke inks 36 em either side, so
        // 38 em of white survives between them — well under the 72 em
        // floor. The third bar exists to lift the row's ink coverage over
        // the dot floor, so that the GAP is what rejects this row and not
        // the coverage rule firing first. That ordering is the point: an
        // invariant that shadows another one makes the shadowed one
        // untestable.
        let text = "demo\tDemo\tG\tS M -450 -250 L 450 -250;S M -450 190 L 450 190;\
                    S M -450 300 L 450 300";
        let err = check_and_flatten(&parse_manifest(text).expect("parses")).expect_err("rejects");
        assert!(matches!(
            err,
            GenerateError::Icon {
                invariant: Invariant::MinFeature,
                ..
            }
        ));
    }

    #[test]
    fn features_that_cross_are_not_a_gap() {
        // The infantry saltire: two strokes crossing at the centre, so
        // every quadrant of the X is white pinching to nothing beside the
        // crossing. Under any distance-between-features rule this shape
        // fails, and it is the shape that most needs to pass. What makes
        // it legal is that the two marks TOUCH, so the white is one
        // shape's own notch rather than a gap between two.
        let text = "demo\tDemo\tG\tS M -450 -450 L 450 450;S M -450 450 L 450 -450";
        let flats = check_and_flatten(&parse_manifest(text).expect("parses")).expect("accepts");
        assert_eq!(flats.len(), 1);
    }

    #[test]
    fn two_features_that_stop_short_are_still_a_gap() {
        // The same saltire, with the crossing pulled apart into a near
        // miss: the marks no longer touch, so the white between their
        // nearest points is a gap the device has to resolve, and it is
        // under the floor. This is the pair that the crossing test above
        // must not accidentally excuse.
        // The second stroke stops at (200, 320): 120/sqrt(2) = 85 em
        // perpendicular from the first stroke's centreline, which leaves
        // 85 - 36 = 49 em of white — about a pixel, over the one cell
        // that counts as merged ink and under the 72 em floor. So the
        // marks never touch, and the white between them is a gap.
        let text = "demo\tDemo\tG\tS M -450 -450 L 450 450;S M -450 450 L 200 320";
        let err = check_and_flatten(&parse_manifest(text).expect("parses")).expect_err("rejects");
        assert!(matches!(
            err,
            GenerateError::Icon {
                invariant: Invariant::MinFeature,
                ..
            }
        ));
    }

    #[test]
    fn a_well_separated_pair_is_accepted() {
        let text = "demo\tDemo\tG\tS M -450 -450 L -450 450;S M 450 -450 L 450 450";
        check_and_flatten(&parse_manifest(text).expect("parses")).expect("accepts");
    }

    #[test]
    fn generation_is_deterministic() {
        // A diff that fires on an unchanged tree is worse than no check.
        assert_eq!(
            generate(WIDE).expect("generates"),
            generate(WIDE).expect("generates")
        );
    }

    #[test]
    fn the_diff_shows_both_sides() {
        let d = unified_diff("a\nb\n", "a\nc\n", Path::new("x.rs"));
        assert!(d.contains("-b"), "{d}");
        assert!(d.contains("+c"), "{d}");
    }

    #[test]
    fn a_flattened_icon_fills_the_unit_square() {
        // Invariant 6's promise, on a shape whose longer axis is known.
        let text = "demo\tDemo\tG\tF M -450 -200 L 450 -200 L 450 200 L -450 200 Z";
        let flats = check_and_flatten(&parse_manifest(text).expect("parses")).expect("accepts");
        let mark = &flats[0].marks[0];
        assert!(mark.filled, "the manifest prefixed this subpath F");
        let (mut x0, mut x1, mut y0, mut y1) = (f32::MAX, f32::MIN, f32::MAX, f32::MIN);
        for p in &mark.pts {
            x0 = x0.min(p.0);
            x1 = x1.max(p.0);
            y0 = y0.min(p.1);
            y1 = y1.max(p.1);
        }
        assert!((x1 - x0 - 1.0).abs() < 1e-4, "width {}", x1 - x0);
        assert!(((x0 + x1) / 2.0 - 0.5).abs() < 1e-4);
        assert!(((y0 + y1) / 2.0 - 0.5).abs() < 1e-4);
        // Uniform scale, so the short axis keeps its aspect rather than
        // being stretched into the hostile frame's rhombus.
        assert!(((y1 - y0) / (x1 - x0) - 400.0 / 900.0).abs() < 1e-3);
    }
}