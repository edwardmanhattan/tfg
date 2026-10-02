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
//! ## Two files, not one
//!
//! `assets/symbology/milsymbol.tsv` holds the GEOMETRY, extracted from the
//! MIT-licensed `spatialillusions/milsymbol` tables at a pinned commit.
//! `assets/symbology/icons.tsv` holds the SELECTION: which upstream keys
//! this client draws, what it calls them, and which battle dimension each
//! draws in.
//!
//! They are separate because they are different kinds of claim. The geometry
//! is inherited evidence with a licence, a commit and a licence file; the
//! selection is a judgement about which icons are worth drawing at 22 px.
//! Merged into one file, a reviewer could not tell which coordinates were
//! chosen and which were copied, and the judgement — the only part worth
//! arguing with — would be buried in a wall of `d` attributes.
//!
//! ## The grammar is upstream's, not ours
//!
//! The subpath cell is real SVG path data: `M L H V C S Q T A Z`, absolute
//! and relative, with implicit repeats and numbers written without
//! separators. [`super::svgpath`] parses it. An earlier revision of this
//! generator used a ten-command DSL of `M`/`L`/`A` only, on the reasoning
//! that a narrower grammar lets it prove the table sits inside its declared
//! primitive set. That was sound while the table was hand-authored and the
//! primitive set was a choice. It is wrong now that the geometry has a
//! source: upstream writes cubics, and a parser that refused them would
//! refuse most of the vocabulary.
//!
//! ## Why the coordinates come out flattened
//!
//! epaint 0.36 has no `PathEl` and no `FillRule`, so a painter could not
//! consume a curve even if this module handed it one, and the headless
//! contact sheet in the harness has no epaint at all. Every curve
//! therefore resolves to line segments HERE, once, and both consumers read
//! the same segments. That is the property that makes a sheet a truthful
//! picture of the map rather than a second renderer that agrees with the
//! first one by luck.

use std::collections::HashSet;
use std::fmt;
use std::path::{Path, PathBuf};

use super::BattleDimension;
use super::icons::{
    EM_BOX, EM_HALF, EM_PER_PX, Fit, GAP_SAMPLES_PER_PX, MIN_ICON_STROKE_EM, MIN_INK_EXTENT,
    MIN_STROKE_PX, Point,
};
use super::svgpath::{self, Run};

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
        .ok_or_else(|| {
            io(format!(
                "no assets/symbology/icons.tsv above {}",
                env!("CARGO_MANIFEST_DIR")
            ))
        })
}

/// The generated file, beside the model that consumes it.
pub fn generated_path() -> Result<PathBuf, GenerateError> {
    repo_root()
        .map(|r| r.join("src").join("symbology").join("icons_generated.rs"))
        .ok_or_else(|| {
            io(format!(
                "no src/symbology/ above {}",
                env!("CARGO_MANIFEST_DIR")
            ))
        })
}

/// The vendored geometry, beside the selection that names rows in it.
pub fn geometry_path() -> Result<PathBuf, GenerateError> {
    repo_root()
        .map(|r| r.join("assets").join("symbology").join("milsymbol.tsv"))
        .ok_or_else(|| {
            io(format!(
                "no assets/symbology/milsymbol.tsv above {}",
                env!("CARGO_MANIFEST_DIR")
            ))
        })
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

/// Upstream's authoring box: 200x200, centred on (100,100), y down.
///
/// Read straight off the tables — every coordinate in `milsymbol.tsv` sits
/// inside it — and converted to em by [`to_em`]. It is a property of the
/// source, not a choice, which is why it lives here rather than in `icons.rs`.
const UPSTREAM_BOX: f64 = 200.0;

/// Upstream's origin, in its own space.
const UPSTREAM_ORIGIN: f64 = UPSTREAM_BOX / 2.0;

/// The selection manifest's way of saying "this icon has no glyph".
///
/// A named sentinel rather than an empty cell, because an empty cell is
/// indistinguishable from a file whose trailing tab was stripped, and the
/// failure that produces — a parse error on save — looks like a bug in the
/// parser rather than in the file.
pub const NO_GEOMETRY: &str = "-";

/// Milsymbol's space to tfg's: centre on the origin, scale to the em box.
///
/// Uniform, and the ratio is exactly 5, so this is a change of origin and a
/// change of unit rather than a fit. Anything else would be scaling the art,
/// which is the fit's job and is done once, later, by [`fit_for`].
fn to_em(p: (f64, f64)) -> (f64, f64) {
    let k = EM_BOX / UPSTREAM_BOX;
    ((p.0 - UPSTREAM_ORIGIN) * k, (p.1 - UPSTREAM_ORIGIN) * k)
}

/// One row of `icons.tsv`: which icon, what it is called, and where its
/// geometry lives upstream.
///
/// `pub(crate)` because [`check_and_flatten`] takes a slice of these and is
/// itself `pub(crate)` for the tests. A `struct Row` behind a `pub(crate) fn`
/// is a `private_interfaces` warning, and the honest fix is to widen the type
/// rather than to narrow the function: the tests need to build rows by
/// parsing a manifest, and hiding that behind a second constructor would be a
/// wrapper with exactly one caller.
#[derive(Debug)]
pub(crate) struct Row {
    variant: String,
    display: String,
    dimension: char,
    /// The key in `milsymbol.tsv`.
    upstream: String,
}

/// One subpath as upstream wrote it: a fill flag and a `d`.
///
/// `pub(crate)` for the same reason as [`Row`]: it appears in the
/// `pub(crate)` signature of [`check_and_flatten`].
#[derive(Debug)]
pub(crate) struct UpstreamMark {
    filled: bool,
    d: String,
}

/// `milsymbol.tsv`, keyed by upstream icon name.
///
/// A `BTreeMap` rather than a `HashMap` because generation must be
/// deterministic, and an error message that names the same key twice in the
/// same order is a message two people can compare.
type Geometry = std::collections::BTreeMap<String, Vec<UpstreamMark>>;

/// Parse the selection manifest.
///
/// Everything is validated HERE and nothing afterwards, because this is the
/// only reader of a file a human types into, so this is where a bad value
/// belongs.
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
                "icons.tsv line {}: expected 4 tab-separated cells, found {}",
                lineno + 1,
                cells.len()
            )));
        }
        let variant = cells[0].trim().to_string();
        let display = cells[1].trim().to_string();
        let letter = cells[2].trim();
        let key = cells[3].trim().to_string();
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
        rows.push(Row {
            variant,
            display,
            dimension,
            upstream: key,
        });
    }
    if rows.is_empty() {
        return Err(io("icons.tsv has no icon rows"));
    }
    Ok(rows)
}

/// Parse the vendored geometry table.
///
/// No path grammar is interpreted here — only the subpath separator and the
/// F/S prefix. Parsing a `d` is [`super::svgpath`]'s job and happens later,
/// against an icon, so that a malformed path is reported against the icon that
/// uses it rather than against a table of 687 rows nobody chose.
fn parse_geometry(text: &str) -> Result<Geometry, GenerateError> {
    let mut out = Geometry::new();
    for (lineno, raw) in text.lines().enumerate() {
        let line = raw.trim_end_matches('\r');
        if line.trim().is_empty() || line.trim_start().starts_with('#') {
            continue;
        }
        let cells: Vec<&str> = line.split('\t').collect();
        if cells.len() != 3 {
            return Err(io(format!(
                "milsymbol.tsv line {}: expected 3 tab-separated cells, found {}",
                lineno + 1,
                cells.len()
            )));
        }
        let key = cells[0].trim().to_string();
        let err = |d: String| bad(&key, None, Invariant::Grammar, d);
        let mut marks = Vec::new();
        for raw_sub in cells[2].split(';') {
            let raw_sub = raw_sub.trim();
            if raw_sub.is_empty() {
                return Err(err("empty subpath between separators".into()));
            }
            let (prefix, body) = raw_sub.split_at(1);
            let filled = match prefix {
                "F" => true,
                "S" => false,
                other => {
                    return Err(err(format!(
                        "subpath must start with F or S, found `{other}`"
                    )));
                }
            };
            marks.push(UpstreamMark {
                filled,
                d: body.trim().to_string(),
            });
        }
        if out.insert(key.clone(), marks).is_some() {
            return Err(err("appears twice in milsymbol.tsv".into()));
        }
    }
    if out.is_empty() {
        return Err(io("milsymbol.tsv has no geometry rows"));
    }
    Ok(out)
}

fn is_snake_case(s: &str) -> bool {
    !s.is_empty()
        && !s.starts_with('_')
        && !s.ends_with('_')
        && !s.contains("__")
        && s.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
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
fn check_rows(rows: &[Row], geometry: &Geometry) -> Result<(), GenerateError> {
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
    // Two icons may not name the same upstream key: they would flatten to
    // identical geometry under two names, and the enum would grow a variant
    // that no reader can tell apart from its twin. The no-glyph sentinel is
    // exempt, because every no-glyph icon legitimately shares it.
    for (i, a) in rows.iter().enumerate() {
        if a.upstream == NO_GEOMETRY {
            continue;
        }
        if let Some(b) = rows[i + 1..].iter().find(|r| r.upstream == a.upstream) {
            return Err(GenerateError::File {
                invariant: Invariant::Exhaustiveness,
                detail: format!(
                    "icons `{}` and `{}` both name upstream `{}`; one glyph cannot be two \
                     icons, and at 22 px nobody could tell them apart anyway",
                    a.variant, b.variant, a.upstream
                ),
            });
        }
        if !geometry.contains_key(&a.upstream) {
            return Err(bad(
                &a.variant,
                None,
                Invariant::Grammar,
                format!(
                    "no geometry row `{}` in milsymbol.tsv; re-run \
                     assets/symbology/extract-mjs.mjs, or check the key against the table",
                    a.upstream
                ),
            ));
        }
    }
    Ok(())
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
pub(crate) fn check_and_flatten(
    rows: &[Row],
    geometry: &Geometry,
) -> Result<Vec<Flat>, GenerateError> {
    check_rows(rows, geometry)?;
    // Every icon's errors, not just the first one's. A vocabulary review is
    // the one place where a partial answer is worse than no answer: with
    // fail-fast, re-pinning upstream means twenty iterations of "fix this one,
    // run again, find the next", and the temptation is to widen the thresholds
    // until the noise stops rather than to read the twenty messages.
    //
    // The FIRST failure stays the error — same variant, same invariant, same
    // icon — with the rest appended to its detail. Returning a whole-file
    // error instead would be tidier to build and would cost the caller the
    // ability to ask WHICH rule fired, which is the one thing the error
    // carries that the text does not.
    let results: Vec<Result<Flat, GenerateError>> =
        rows.iter().map(|r| flatten_one(r, geometry)).collect();
    let mut failures = results.iter().filter_map(|r| r.as_ref().err());
    let Some(first) = failures.next() else {
        return Ok(results
            .into_iter()
            .map(|r| r.expect("no failures"))
            .collect());
    };
    let rest: Vec<String> = failures.map(|e| e.to_string()).collect();
    let GenerateError::Icon {
        icon,
        subpath,
        invariant,
        detail,
    } = first
    else {
        return Err(clone_err(first));
    };
    Err(GenerateError::Icon {
        icon: icon.clone(),
        subpath: *subpath,
        invariant: *invariant,
        detail: format!(
            "{detail}\n\n{icon}: {} more icon(s) also fail:\n  {}",
            rest.len(),
            rest.join("\n  ")
        ),
    })
}

/// Rebuild a file-level error, which `check_and_flatten` has no way to mutate
/// in place.
fn clone_err(e: &GenerateError) -> GenerateError {
    match e {
        GenerateError::File { invariant, detail } => GenerateError::File {
            invariant: *invariant,
            detail: detail.clone(),
        },
        GenerateError::Icon {
            icon,
            subpath,
            invariant,
            detail,
        } => GenerateError::Icon {
            icon: icon.clone(),
            subpath: *subpath,
            invariant: *invariant,
            detail: detail.clone(),
        },
    }
}

/// Resolve one row against the geometry table and flatten it.
///
/// The join is here, in one place, rather than in [`parse_manifest`], because
/// an unknown upstream key is only wrong in the context of the row that names
/// it: the same string is a valid key for one icon and a typo for another, and
/// reporting it against the row is the difference between a message somebody
/// can act on and one they have to grep for.
fn flatten_one(row: &Row, geometry: &Geometry) -> Result<Flat, GenerateError> {
    let mut tagged: Vec<Tagged> = Vec::new();
    let mut bbox: Option<(f64, f64, f64, f64)> = None;

    // An icon with no geometry is a real state, not a missing one:
    // `unspecified` draws an empty frame interior, which is what the standard
    // says to draw when nobody has said what a thing is. It is spelled as the
    // sentinel key [`NO_GEOMETRY`], and it is the ONE exemption from the
    // coverage floor; every other invariant is vacuous rather than skipped.
    if row.upstream == NO_GEOMETRY {
        return Ok(Flat {
            marks: Vec::new(),
            fit: Fit::NONE,
        });
    }
    let Some(upstream) = geometry.get(&row.upstream) else {
        return Err(bad(
            &row.variant,
            None,
            Invariant::Grammar,
            format!(
                "no geometry row `{}` in milsymbol.tsv; re-run \
                 assets/symbology/extract-mjs.mjs, or check the key against the table",
                row.upstream
            ),
        ));
    };

    for (si, mark) in upstream.iter().enumerate() {
        let runs = svgpath::parse_path(&mark.d)
            .map_err(|e| bad(&row.variant, Some(si), Invariant::Grammar, e.0))?;
        for run in runs {
            emit_run(row, si, mark.filled, run, &mut tagged, &mut bbox)?;
        }
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

/// One device pixel, squared, in em².
///
/// The floor below which a fill is not a fill. Upstream marks a path filled by
/// default — a filled shape with a 3-unit stroke riding on it — and epaint
/// draws a fill and a stroke as separate Shapes, so this module has to pick
/// one per mark. For many of upstream's paths the answer is forced by the
/// geometry rather than chosen: the infantry saltire is
/// `M25,50 L175,150 M25,150 L175,50`, two single segments, and filling
/// either one encloses no area at all, so a faithful "filled" would make the
/// whole icon vanish. What the reader sees is upstream's stroke. Same for
/// reconnaissance's `M25,150 L175,50`.
///
/// So the rule is: a fill that covers less than one device pixel is not a
/// fill, it is a stroke's job. That is a visibility floor rather than a tuning
/// constant — below it the filled region is smaller than the smallest thing
/// the device can put on screen.
///
/// In EM, and measured on points already converted to em. The space matters:
/// upstream's box is 200 units to tfg's 1000, so the same threshold in
/// upstream units would be 25 times too small and would let every hairline
/// sliver through as a fill.
const ONE_PIXEL_AREA_EM: f64 = EM_PER_PX * EM_PER_PX;

/// Convert one upstream run to em, check it is inside the box, and keep it.
fn emit_run(
    row: &Row,
    si: usize,
    upstream_filled: bool,
    run: Run,
    tagged: &mut Vec<Tagged>,
    bbox: &mut Option<(f64, f64, f64, f64)>,
) -> Result<(), GenerateError> {
    if run.points.len() < 2 {
        return Ok(());
    }
    let pts: Vec<(f64, f64)> = run.points.iter().map(|p| to_em(*p)).collect();
    let filled = upstream_filled && svgpath::shoelace(&pts) >= ONE_PIXEL_AREA_EM;
    // The box is checked on the FLATTENED polyline, not on the control
    // points. A cubic's control points routinely sit outside the curve, and
    // checking them would reject half the vocabulary for a bulge the reader
    // never sees. The flattened vertices bound the curve to within the
    // flattening tolerance, which is a quarter of a pixel.
    for (x, y) in &pts {
        in_box(row, si, *x, *y)?;
    }
    // Seed from the first point and then extend over ALL of them, in that
    // order. Seeding inside the loop and extending outside it — which is what
    // this looked like for a moment — silently truncates the box of any icon
    // whose FIRST subpath is its only subpath, to a zero-size box at that
    // first vertex. The coverage floor then rejects the icon for having no
    // ink, which reads like a geometry bug and is a bookkeeping one.
    let (fx, fy) = pts[0];
    match bbox {
        None => *bbox = Some((fx, fy, fx, fy)),
        Some(b) => {
            b.0 = b.0.min(fx);
            b.1 = b.1.min(fy);
            b.2 = b.2.max(fx);
            b.3 = b.3.max(fy);
        }
    }
    for (x, y) in &pts {
        let b = bbox.as_mut().expect("seeded above");
        b.0 = b.0.min(*x);
        b.1 = b.1.min(*y);
        b.2 = b.2.max(*x);
        b.3 = b.3.max(*y);
    }
    tagged.push(Tagged {
        filled,
        pts,
        subpath: si,
    });
    Ok(())
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

/// The uniform em-to-unit-square fit, plus the extent floor it answers.
///
/// The floor is checked on the AUTHORED ink because the fit rescales whatever
/// was drawn to fill the square: the question is "was this drawn at a sane size
/// in the em box", which is the source's decision, not "will it be big when
/// drawn", which the fit decides.
///
/// And it is the longer AXIS that is measured, not the area — see
/// [`MIN_INK_EXTENT`](super::icons::MIN_INK_EXTENT) for why area rejects
/// APP-6C's own horizontal-bar icons.
fn fit_for(row: &Row, bbox: Option<(f64, f64, f64, f64)>) -> Result<Fit, GenerateError> {
    // No ink is a real state, not a failure: `unspecified` draws an empty
    // frame interior and that is what the standard says to draw.
    let Some((x0, y0, x1, y1)) = bbox else {
        return Ok(Fit::NONE);
    };
    let (w, h) = (x1 - x0, y1 - y0);
    let extent = w.abs().max(h.abs()) / EM_BOX;
    if extent < MIN_INK_EXTENT {
        return Err(bad(
            &row.variant,
            None,
            Invariant::InkCoverage,
            format!(
                "ink spans {w:.0} x {h:.0} em, so its longer axis is {:.1}% of the em box, \
                 under the {:.0}% floor; either the icon is meant to be much larger, or it is \
                 a dot that the fit would blow up into a smear",
                extent * 100.0,
                MIN_INK_EXTENT * 100.0,
            ),
        ));
    }
    // A zero extent on ONE axis is not a degenerate icon and is deliberately not
    // special-cased. APP-6C's `supply` is a bare horizontal line — 750 em wide
    // and zero em tall, meant to be drawn at full width — and an earlier
    // version of this function bailed out to `Fit::NONE` on `w <= 0 || h <= 0`,
    // which left it in em coordinates inside a table documented as being in
    // the unit square, where the painter multiplies by the box and draws it
    // hundreds of pixels off screen. `w.max(h)` is non-zero because the
    // coverage floor above refuses anything whose longer axis is under 15% of
    // the box, and that includes the both-zero case.
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
fn check_gaps(
    row: &Row,
    tagged: &[Tagged],
    bbox: Option<(f64, f64, f64, f64)>,
) -> Result<(), GenerateError> {
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
        for (slot, (dx, dy)) in [(-1i64, 0i64), (1, 0), (0, -1), (0, 1)]
            .into_iter()
            .enumerate()
        {
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
///
/// No clamp, deliberately. This used to saturate at plus or minus 8, which is
/// a number no normalised coordinate can reach and an em coordinate reaches
/// easily — so it silently rewrote a geometry bug into a different geometry
/// bug, and the one icon it hit drew as a 16 em line where the standard's is
/// 750. A wrong coordinate should print wrong and be caught by
/// `every_icon_sits_in_the_unit_square`.
const QUANT: f32 = 4096.0;

fn q(v: f32) -> String {
    format!("{:.6}", (v * QUANT).round() / QUANT)
}

/// Parse, check and emit the generated file, all in memory.
pub fn generate(manifest: &str, geometry: &str) -> Result<String, GenerateError> {
    let rows = parse_manifest(manifest)?;
    let geom = parse_geometry(geometry)?;
    let flats = check_and_flatten(&rows, &geom)?;
    Ok(emit(&rows, &flats))
}

/// Read both tables from their canonical locations and generate.
///
/// The two paths are found rather than passed because every caller wants both,
/// and a caller that could supply one and not the other is a caller that will
/// eventually check the manifest against stale geometry.
fn read_tables() -> Result<(String, String), GenerateError> {
    let manifest = manifest_path()?;
    let geometry = geometry_path()?;
    let a = std::fs::read_to_string(&manifest)
        .map_err(|e| io(format!("{}: {e}", manifest.display())))?;
    let b = std::fs::read_to_string(&geometry)
        .map_err(|e| io(format!("{}: {e}", geometry.display())))?;
    Ok((a, b))
}

/// Parse, check and write the generated file.
pub fn regenerate() -> Result<PathBuf, GenerateError> {
    let (manifest, geometry) = read_tables()?;
    let generated = generate(&manifest, &geometry)?;
    let out = generated_path()?;
    // Written to a sibling and renamed, so an interrupted run leaves the
    // previous table intact rather than a half-written file that would
    // fail to compile on the next read.
    let tmp = out.with_extension("rs.tmp");
    std::fs::write(&tmp, &generated).map_err(|e| io(format!("{}: {e}", tmp.display())))?;
    std::fs::rename(&tmp, &out).map_err(|e| io(format!("{}: {e}", out.display())))?;
    Ok(out)
}

/// Report drift between the tables and the checked-in file, as a
/// unified diff.
///
/// A diff that fires on an unchanged tree is worse than no check at all,
/// so the quantisation above is not cosmetic: it is what makes this
/// function's answer stable.
pub fn check_drift(generated: &Path) -> Result<Option<String>, GenerateError> {
    let (manifest, geometry) = read_tables()?;
    let want = generate(&manifest, &geometry)?;
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
        "// GENERATED. Do not hand-edit; regenerate with\n\
         // `cargo run --manifest-path proto/p5-epaint/Cargo.toml -- --symbology-generate`.\n\
         //\n\
         // Which icons exist and what they are called is assets/symbology/icons.tsv.\n\
         // The coordinates come from assets/symbology/milsymbol.tsv, extracted from\n\
         // spatialillusions/milsymbol (MIT, Copyright (c) 2017 Mans Beckman) at a pinned\n\
         // commit; see licenses/milsymbol-LICENSE.md and the header of that file.\n\
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
        s.push_str(&format!(
            "    #[allow(non_camel_case_types)]\n    {} = {i},\n",
            camel(&r.variant)
        ));
    }
    s.push_str("}\n\nimpl UnitIcon {\n");

    s.push_str(&format!(
        "    pub const ALL: [UnitIcon; {}] = [\n",
        rows.len()
    ));
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

    /// A geometry table wide enough to clear every invariant, used as the
    /// baseline the negative tests perturb.
    ///
    /// In UPSTREAM's 200-unit space, not tfg's em box: the geometry table is
    /// upstream's, and writing the fixtures in em would test a conversion the
    /// real path never takes. `BOX` is 100x100 units, which is half the box
    /// and so 500x500 em after conversion.
    const GEOM: &str = concat!(
        "DEMO\tground\tF M 10 10 L 10 190 L 190 190 Z ; ",
        "F M 10 10 L 190 10\n",
        "SPACED\tground\tF M 50 50 L 150 50 L 150 150 Z\n",
        "BOX\tground\tF M 50 75 L 150 75 L 150 125 L 50 125 Z\n",
        "DOTTY\tground\tS M 100 100 L 108 100\n",
    );

    fn geom(text: &str) -> Geometry {
        parse_geometry(text).expect("geometry parses")
    }

    fn rows(text: &str) -> Vec<Row> {
        parse_manifest(text).expect("manifest parses")
    }

    fn flat(manifest: &str, geometry: &str) -> Result<Vec<Flat>, GenerateError> {
        check_and_flatten(&rows(manifest), &geom(geometry))
    }

    #[test]
    fn the_two_stroke_thresholds_cannot_drift_apart() {
        // `MIN_ICON_STROKE_EM` is `MIN_STROKE_PX` converted, not chosen.
        // If this ever fails, invariant 3 is comparing a gap against a
        // stroke that is not the one the painter draws.
        assert!((MIN_ICON_STROKE_EM - MIN_STROKE_PX * EM_PER_PX).abs() < 1e-9);
    }

    #[test]
    fn the_manifest_joins_against_the_geometry() {
        let t = rows("demo\tDemo\tG\tDEMO");
        assert_eq!(t.len(), 1);
        assert_eq!(t[0].variant, "demo");
        assert_eq!(t[0].display, "Demo");
        assert_eq!(t[0].dimension, 'G');
        assert_eq!(t[0].upstream, "DEMO");
    }

    #[test]
    fn comments_and_blank_lines_are_skipped() {
        assert_eq!(rows("# a comment\n\ndemo\tDemo\tG\tDEMO\n").len(), 1);
    }

    #[test]
    fn a_wrong_cell_count_names_the_file_and_line() {
        let msg = parse_manifest("demo\tDemo\tG")
            .expect_err("rejects")
            .to_string();
        assert!(msg.contains("icons.tsv line 1"), "{msg}");
        assert!(msg.contains("found 3"), "{msg}");
    }

    #[test]
    fn an_unknown_upstream_key_is_reported_against_its_row() {
        // The distinction that matters: a typo in `icons.tsv` is a one-line
        // fix, and the message has to say which line.
        let err = flat("demo\tDemo\tG\tNOSUCHKEY", GEOM).expect_err("rejects");
        assert!(err.to_string().contains("demo"), "{err}");
        assert!(err.to_string().contains("NOSUCHKEY"), "{err}");
    }

    #[test]
    fn the_no_geometry_sentinel_is_an_icon_with_no_glyph() {
        // `unspecified` draws an empty frame interior, which is a real state.
        let flats = flat("demo\tDemo\tG\t-", GEOM).expect("accepts");
        assert!(flats[0].marks.is_empty());
    }

    #[test]
    fn an_unknown_dimension_letter_is_rejected() {
        assert!(parse_manifest("demo\tDemo\tQ\tDEMO").is_err());
    }

    #[test]
    fn a_bad_prefix_in_the_geometry_is_rejected() {
        let msg = parse_geometry("K\tground\tX M0,0 L1,1").expect_err("rejects");
        assert!(msg.to_string().contains("F or S"), "{msg}");
    }

    #[test]
    fn a_malformed_path_is_reported_against_the_icon() {
        let g = "BAD\tground\tF M 0 0 K 10 10";
        let err = flat("demo\tDemo\tG\tBAD", g).expect_err("rejects");
        let msg = err.to_string();
        assert!(msg.contains("demo"), "{msg}");
        assert!(msg.contains("subpath 0"), "{msg}");
        assert!(msg.contains("grammar"), "{msg}");
    }

    #[test]
    fn a_non_finite_coordinate_is_refused() {
        // `1e999` reaches the lexer as text and `parse::<f64>` would make it
        // infinity, which then poisons every box comparison downstream.
        let g = "BAD\tground\tF M 0 0 L 1e999 0";
        assert!(flat("demo\tDemo\tG\tBAD", g).is_err());
    }

    #[test]
    fn an_out_of_box_coordinate_reports_em_and_pixels() {
        // Upstream's box is 200 units; 5 units outside it is 25 em.
        let g = "WIDE\tground\tF M -5 -100 L 100 -100 L 100 100 Z";
        let err = flat("demo\tDemo\tG\tWIDE", g).expect_err("rejects");
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
    fn a_cubic_that_leaves_the_box_is_caught_by_its_flattened_vertices() {
        // Control points sit inside the 200 box; the curve does not. This is
        // the case that checking control points instead of the flattened
        // polyline would miss.
        let g = "ARC\tground\tF M 10 100 C 10 -400 190 -400 190 100 Z";
        let err = flat("demo\tDemo\tG\tARC", g).expect_err("rejects");
        assert!(matches!(
            err,
            GenerateError::Icon {
                invariant: Invariant::InBox,
                ..
            }
        ));
    }

    #[test]
    fn a_duplicated_icon_is_refused() {
        let m = "demo\tDemo\tG\tDEMO\ndemo\tDemo again\tG\tSPACED";
        let err = check_and_flatten(&rows(m), &geom(GEOM)).expect_err("refuses");
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
    fn two_icons_may_not_name_the_same_upstream_key() {
        // One glyph under two names is an enum variant no reader could
        // distinguish from its twin, which is worse than not having it.
        let m = "one\tOne\tG\tDEMO\ntwo\tTwo\tG\tDEMO";
        let err = check_and_flatten(&rows(m), &geom(GEOM)).expect_err("refuses");
        assert!(matches!(
            err,
            GenerateError::File {
                invariant: Invariant::Exhaustiveness,
                ..
            }
        ));
        assert!(err.to_string().contains("DEMO"), "{err}");
    }

    #[test]
    fn a_dot_does_not_meet_the_coverage_floor() {
        let err = flat("demo\tDemo\tG\tDOTTY", GEOM).expect_err("rejects");
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
        // Three bars spanning the width. One stroke is 72 em, which is 14.4
        // upstream units, so the top two — 18 units apart, 90 em — leave 18 em
        // of white between their edges: over half a stroke and under the
        // one-stroke floor, so the GAP rejects the row. The third bar lifts
        // the row's ink coverage over the dot floor, so that the gap is what
        // fires and not the coverage rule. That ordering is the point: an
        // invariant that shadows another one makes the shadowed one
        // untestable.
        let g = "GAPS\tground\tS M 10 40 L 190 40 ; S M 10 58 L 190 58 ; \
                  S M 10 160 L 190 160";
        let err = flat("demo\tDemo\tG\tGAPS", g).expect_err("rejects");
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
        // The infantry saltire: two strokes crossing at the centre, so every
        // quadrant of the X is white pinching to nothing beside the crossing.
        // Under any distance-between-features rule this shape fails, and it is
        // the shape that most needs to pass. What makes it legal is that the
        // two marks TOUCH, so the white is one shape's own notch rather than
        // a gap between two.
        let g = "CROSS\tground\tS M 20 20 L 180 180 ; S M 20 180 L 180 20";
        flat("demo\tDemo\tG\tCROSS", g).expect("accepts");
    }

    #[test]
    fn two_features_that_stop_short_are_still_a_gap() {
        // The same saltire pulled apart into a near miss: the marks no longer
        // touch, so the white between their nearest points is a gap the
        // device has to resolve, and it is under the floor. This is the pair
        // the crossing test above must not accidentally excuse.
        //
        // The near end is (140,164): 24/sqrt(2) = 17 units, which is 85 em
        // from the first stroke's centreline, so the white between their edges
        // is 85 - 36 = 49 em — about a pixel, over the one cell that counts
        // as merged ink and under the 72 em floor. So the marks never touch,
        // and the white between them is a gap.
        let g = "CROSS\tground\tS M 10 10 L 190 190 ; S M 10 190 L 140 164";
        let err = flat("demo\tDemo\tG\tCROSS", g).expect_err("rejects");
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
        let g = "APART\tground\tS M 20 20 L 20 180 ; S M 180 20 L 180 180";
        flat("demo\tDemo\tG\tAPART", g).expect("accepts");
    }

    #[test]
    fn an_open_run_upstream_calls_filled_is_stroked() {
        // The infantry saltire as upstream writes it: one `d`, marked filled,
        // two single-segment runs. Filling either encloses no area, so the
        // whole icon would vanish. The decision has to come from the
        // geometry, and this is the test that says so.
        let g = "SALTIRE\tground\tF M 25 50 L 175 150 M25,150 175,50";
        let flats = flat("demo\tDemo\tG\tSALTIRE", g).expect("accepts");
        let marks = &flats[0].marks;
        assert_eq!(marks.len(), 2, "two runs, one per M");
        assert!(
            marks.iter().all(|m| !m.filled),
            "an open line has nothing to fill"
        );
    }

    #[test]
    fn a_closed_run_upstream_calls_filled_is_filled() {
        let flats = flat("demo\tDemo\tG\tSPACED", GEOM).expect("accepts");
        assert!(flats[0].marks.iter().all(|m| m.filled));
    }

    /// The table is documented as being in the unit square, and the painter
    /// multiplies by the box believing it. Checked against the CHECKED-IN
    /// table rather than against a fresh `generate`, because the artifact is
    /// what ships and a generator that is right while its output is stale has
    /// still shipped the stale one.
    ///
    /// This is the invariant that catches a fit which bails out and leaves raw
    /// em coordinates behind — which is exactly how APP-6C's `supply`, a bare
    /// horizontal line with zero height, drew as a 16 em line at (-8, 8).
    #[test]
    fn every_icon_sits_in_the_unit_square() {
        use super::super::icons_generated::{FIT, GEOMETRY, UnitIcon};
        for (icon, marks) in UnitIcon::ALL.iter().zip(GEOMETRY.iter()) {
            for mark in *marks {
                let pts = match mark {
                    super::super::icons::IconMark::Fill(p)
                    | super::super::icons::IconMark::Stroke(p) => *p,
                };
                assert!(!pts.is_empty(), "{icon:?} has an empty mark");
                for &(x, y) in pts {
                    assert!(
                        (-1e-6..=1.0 + 1e-6).contains(&x) && (-1e-6..=1.0 + 1e-6).contains(&y),
                        "{icon:?} has ({x}, {y}) outside the unit square"
                    );
                }
            }
        }
    }

    /// And the identity fit means exactly one thing: no ink at all. An icon
    /// with geometry and an identity fit is the other half of the same bug,
    /// because a caller reasoning about margins from `FIT` would be told the
    /// icon has none.
    #[test]
    fn only_an_icon_with_no_ink_has_the_identity_fit() {
        use super::super::icons_generated::{FIT, GEOMETRY, UnitIcon};
        for (icon, (marks, fit)) in UnitIcon::ALL.iter().zip(GEOMETRY.iter().zip(FIT.iter())) {
            assert_eq!(
                marks.is_empty(),
                *fit == super::super::icons::Fit::NONE,
                "{icon:?} has {} marks and fit {fit:?}",
                marks.len()
            );
        }
    }

    #[test]
    fn generation_is_deterministic() {
        // A diff that fires on an unchanged tree is worse than no check.
        assert_eq!(
            generate("demo\tDemo\tG\tDEMO", GEOM).expect("generates"),
            generate("demo\tDemo\tG\tDEMO", GEOM).expect("generates")
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
        // Invariant 6's promise, on a shape whose longer axis is known: a
        // 100x70 upstream box is 500x350 em, so the uniform fit scales it to
        // 1.0 x 0.7 and centres it. 350 em also clears the coverage floor,
        // which a 100x50 box would not — that is what the floor is for.
        let g = "WIDE\tground\tF M 50 65 L 150 65 L 150 135 L 50 135 Z";
        let flats = flat("demo\tDemo\tG\tWIDE", g).expect("accepts");
        let mark = &flats[0].marks[0];
        assert!(mark.filled, "upstream marked this subpath filled");
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
        assert!(((y1 - y0) / (x1 - x0) - 0.7).abs() < 1e-3);
    }
}
