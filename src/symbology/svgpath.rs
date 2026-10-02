//! A real SVG path parser: the commands, the number lexer, and the curve
//! flattening.
//!
//! It exists because the icon geometry now has a source. `assets/symbology/
//! milsymbol.tsv` carries the paths as upstream wrote them, and upstream is a
//! JavaScript library drawing into a browser canvas, so the data is full SVG
//! path grammar: `M L H V C S Q T A Z`, absolute and relative, with implicit
//! repeats and numbers written without separators (`m25,50l150,0`). A parser
//! that accepted only `M`, `L` and `A` would be a parser for a language
//! nobody upstream writes in.
//!
//! Curves do NOT survive into the output. epaint 0.36 has no `PathEl` and no
//! `FillRule` — `PathShape` is one `Vec<Pos2>` documented convex-only — so a
//! painter could not consume a bezier even if this handed it one, and the
//! headless contact sheet in `proto/p5-epaint` has no epaint at all. Every
//! curve therefore resolves to line segments HERE, once, and both consumers
//! read the same segments. That is what makes a sheet a truthful picture of the
//! map rather than a second renderer that agrees with the first by luck.
//!
//! The flattening tolerance is [`super::icons::MIN_ARC_SAGITTA_EM`], the same
//! quarter-of-a-device-pixel figure the sagitta invariant is stated in. One
//! number for both, because "as smooth as the device can show" and "smooth
//! enough not to trip the sagitta floor" are the same requirement, and two
//! constants would be two answers to it.

use super::icons::MIN_ARC_SAGITTA_EM;

/// One point in the path's own coordinate space.
type P = (f64, f64);

/// What went wrong, and where.
///
/// A flat `String` rather than a rich error type: the caller already knows the
/// icon and the subpath it is reporting against, and it wraps this in
/// [`crate::symbology::generate::GenerateError`], which is where the icon name
/// and the invariant live.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathError(pub String);

impl std::fmt::Display for PathError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

fn err<T>(msg: impl Into<String>) -> Result<T, PathError> {
    Err(PathError(msg.into()))
}

/// The largest number of segments one curve may flatten into.
///
/// Not a quality knob. A curve that needs more than this to hold the tolerance
/// is either enormously oversized — the tables' `S` variants reach 205 units
/// across in a 200 box, which is a selection mistake and not a curve problem —
/// or the control points are degenerate. Either way it deserves to be said out
/// loud rather than to emit a thousand-point table nobody will look at.
pub const SEGMENT_CAP: usize = 256;

/// One polyline out of the path, already flattened.
///
/// `closed` says whether an explicit `Z` ended it, which is what tells a
/// filled mark apart from a stroked one. It is NOT the same question as
/// whether the points happen to enclose area; see [`Run::wants_fill`].
#[derive(Debug, Clone, PartialEq)]
pub struct Run {
    pub points: Vec<P>,
    pub closed: bool,
}

impl Run {
    /// Shoelace area of the run treated as an implicitly closed loop.
    ///
    /// The points are in whatever space the path was authored in. The caller
    /// decides whether that space is the one its threshold is stated in; see
    /// [`crate::symbology::generate::ONE_PIXEL_AREA_EM`] for why that
    /// distinction is not pedantic.
    pub fn enclosed_area(&self) -> f64 {
        shoelace(&self.points)
    }
}

/// Shoelace area of a point ring treated as an implicitly closed loop.
///
/// Free rather than a method because the fill decision needs it on points that
/// have already been converted, not on the [`Run`] they came from.
///
/// The wrap-around term is only added when the ring does not already end where
/// it began. A `Z` makes [`parse_path`] append the subpath's first point, so a
/// closed run's last point equals its first and the closing edge is already
/// there; adding it again double-counts it and reports twice the true area.
pub fn shoelace(points: &[(f64, f64)]) -> f64 {
    if points.len() < 3 {
        return 0.0;
    }
    let first = points[0];
    let tail = points[points.len() - 1];
    let already_closed = (first.0 - tail.0).abs() < 1e-9 && (first.1 - tail.1).abs() < 1e-9;
    let last = if already_closed {
        points.len() - 2
    } else {
        points.len() - 1
    };
    let mut acc = 0.0;
    for i in 0..last {
        let a = points[i];
        let b = points[i + 1];
        acc += a.0 * b.1 - b.0 * a.1;
    }
    if !already_closed {
        acc += tail.0 * first.1 - first.0 * tail.1;
    }
    acc.abs() / 2.0
}

/// Parse one `d` attribute into flattened runs.
///
/// A path with no drawing command at all is an error rather than an empty
/// result: every run this function returns has at least two points, so a path
/// that cannot produce one is malformed, and a silent empty result would turn
/// a typo into an icon that draws nothing and passes every invariant.
pub fn parse_path(d: &str) -> Result<Vec<Run>, PathError> {
    let mut lex = Lexer::new(d);
    let mut runs: Vec<Run> = Vec::new();
    let mut cur: Vec<P> = Vec::new();
    let mut closed = false;

    // The current point and the two control points the smooth variants mirror.
    // `None` after a `Z`, because the current point is then the subpath start.
    let mut at: Option<P> = None;
    let mut start: Option<P> = None;
    let mut last_cubic_ctrl: Option<P> = None;
    let mut last_quad_ctrl: Option<P> = None;

    loop {
        let Some(cmd) = lex.peek_cmd() else { break };
        lex.take_cmd();
        let rel = cmd.is_ascii_lowercase();
        let up = cmd.to_ascii_uppercase();

        // The point a relative command measures from. Before any `M` there
        // is no current point, and SVG resolves a leading relative command
        // against the origin — so `m 80,100` is a legal way to write
        // `M 80,100`. Upstream's tables use that form, so rejecting it would
        // reject the vocabulary. After a `Z` the current point is the
        // subpath's first point.
        let origin = at.or(start).unwrap_or((0.0, 0.0));

        match up {
            'M' => {
                let p = lex.point(rel, origin)?;
                flush(&mut runs, &mut cur, &mut closed);
                cur = vec![p];
                closed = false;
                at = Some(p);
                start = Some(p);
                last_cubic_ctrl = None;
                last_quad_ctrl = None;
                // Repeated coordinate pairs after a move are implicit LINES,
                // not more moves. This is the single most commonly got
                // detail in hand-written path code and the tables rely on it.
                while lex.peek_is_number() {
                    let q = lex.point(rel, at.unwrap())?;
                    cur.push(q);
                    at = Some(q);
                }
            }
            'L' => {
                let p = lex.point(rel, origin)?;
                cur.push(p);
                at = Some(p);
                last_cubic_ctrl = None;
                last_quad_ctrl = None;
                while lex.peek_is_number() {
                    let q = lex.point(rel, at.unwrap())?;
                    cur.push(q);
                    at = Some(q);
                }
            }
            'H' => {
                let x = lex.number()?;
                let p = (if rel { origin.0 + x } else { x }, origin.1);
                cur.push(p);
                at = Some(p);
                last_cubic_ctrl = None;
                last_quad_ctrl = None;
                while lex.peek_is_number() {
                    let x = lex.number()?;
                    let q = (if rel { at.unwrap().0 + x } else { x }, at.unwrap().1);
                    cur.push(q);
                    at = Some(q);
                }
            }
            'V' => {
                let y = lex.number()?;
                let p = (origin.0, if rel { origin.1 + y } else { y });
                cur.push(p);
                at = Some(p);
                last_cubic_ctrl = None;
                last_quad_ctrl = None;
                while lex.peek_is_number() {
                    let y = lex.number()?;
                    let q = (at.unwrap().0, if rel { at.unwrap().1 + y } else { y });
                    cur.push(q);
                    at = Some(q);
                }
            }
            'C' => {
                let c1 = lex.point(rel, origin)?;
                let c2 = lex.point(rel, origin)?;
                let p = lex.point(rel, origin)?;
                cubic(&mut cur, origin, c1, c2, p)?;
                at = Some(p);
                last_cubic_ctrl = Some(c2);
                last_quad_ctrl = None;
                while lex.peek_is_number() {
                    let d1 = lex.point(rel, at.unwrap())?;
                    let d2 = lex.point(rel, at.unwrap())?;
                    let e = lex.point(rel, at.unwrap())?;
                    cubic(&mut cur, at.unwrap(), d1, d2, e)?;
                    at = Some(e);
                    last_cubic_ctrl = Some(d2);
                }
            }
            'S' => {
                // The first control is the reflection of the previous cubic's
                // second control. With no previous cubic, or a previous
                // command that was not a cubic, the spec says it COINCIDES
                // with the current point, which makes the curve degenerate to a
                // quadratic rather than an error.
                let reflected = match (last_cubic_ctrl, at) {
                    (Some(c), Some(p)) => (2.0 * p.0 - c.0, 2.0 * p.1 - c.1),
                    _ => origin,
                };
                let c2 = lex.point(rel, origin)?;
                let p = lex.point(rel, origin)?;
                cubic(&mut cur, origin, reflected, c2, p)?;
                at = Some(p);
                last_cubic_ctrl = Some(c2);
                last_quad_ctrl = None;
                while lex.peek_is_number() {
                    let d2 = lex.point(rel, at.unwrap())?;
                    let e = lex.point(rel, at.unwrap())?;
                    let d1 = (2.0 * at.unwrap().0 - c2.0, 2.0 * at.unwrap().1 - c2.1);
                    cubic(&mut cur, at.unwrap(), d1, d2, e)?;
                    at = Some(e);
                    last_cubic_ctrl = Some(d2);
                }
            }
            'Q' => {
                let c = lex.point(rel, origin)?;
                let p = lex.point(rel, origin)?;
                quad(&mut cur, origin, c, p)?;
                at = Some(p);
                last_quad_ctrl = Some(c);
                last_cubic_ctrl = None;
                while lex.peek_is_number() {
                    let d = lex.point(rel, at.unwrap())?;
                    let e = lex.point(rel, at.unwrap())?;
                    quad(&mut cur, at.unwrap(), d, e)?;
                    at = Some(e);
                    last_quad_ctrl = Some(d);
                }
            }
            'T' => {
                let reflected = match (last_quad_ctrl, at) {
                    (Some(c), Some(p)) => (2.0 * p.0 - c.0, 2.0 * p.1 - c.1),
                    _ => origin,
                };
                let p = lex.point(rel, origin)?;
                quad(&mut cur, origin, reflected, p)?;
                at = Some(p);
                last_quad_ctrl = Some(reflected);
                last_cubic_ctrl = None;
                while lex.peek_is_number() {
                    let e = lex.point(rel, at.unwrap())?;
                    let d = (2.0 * at.unwrap().0 - reflected.0, 2.0 * at.unwrap().1 - reflected.1);
                    quad(&mut cur, at.unwrap(), d, e)?;
                    at = Some(e);
                    last_quad_ctrl = Some(d);
                }
            }
            'A' => {
                let rx = lex.number()?;
                let ry = lex.number()?;
                let rot = lex.number()?;
                let large = lex.flag()?;
                let sweep = lex.flag()?;
                let p = lex.point(rel, origin)?;
                arc(&mut cur, origin, rx, ry, rot, large, sweep, p)?;
                at = Some(p);
                last_cubic_ctrl = None;
                last_quad_ctrl = None;
                while lex.peek_is_number() {
                    let (rx2, ry2, rot2, l2, s2, e2) = lex.arc_tail(rel, at.unwrap())?;
                    arc(&mut cur, at.unwrap(), rx2, ry2, rot2, l2, s2, e2)?;
                    at = Some(e2);
                }
            }
            'Z' => {
                if let Some(s) = start {
                    cur.push(s);
                    closed = true;
                }
                flush(&mut runs, &mut cur, &mut closed);
                // The current point is the subpath's first point, so a command
                // after `Z` opens a NEW subpath that has to begin there. Seeding
                // the run with it is what makes `... Z l5 0` draw a line from
                // the start rather than dropping the lone endpoint that would
                // otherwise be all the new run ever holds.
                cur = start.map(|s| vec![s]).unwrap_or_default();
                at = None;
                last_cubic_ctrl = None;
                last_quad_ctrl = None;
            }
            other => return err(format!("unknown path command `{other}`")),
        }
    }

    flush(&mut runs, &mut cur, &mut closed);
    if runs.is_empty() {
        return err("path has no drawable run");
    }
    Ok(runs)
}

fn flush(runs: &mut Vec<Run>, cur: &mut Vec<P>, closed: &mut bool) {
    if cur.len() >= 2 {
        runs.push(Run {
            points: std::mem::take(cur),
            closed: *closed,
        });
    } else {
        cur.clear();
    }
    *closed = false;
}

// --- curve flattening -------------------------------------------------------

/// Segments needed to hold the tolerance, from an upper bound on the
/// deviation of a curve from its chord.
///
/// Both bounds below are the standard second-difference estimates: a cubic's
/// control polygon deviates from the chord by at most a quarter of its
/// second difference, and halving the segment shrinks that by the square, so
/// the count is the square root. Using a bound rather than a measurement is
/// the point — the answer must not depend on how the curve happens to be
/// oriented.
fn cubic_segments(p0: P, p1: P, p2: P, p3: P) -> usize {
    let d = |a: P, b: P, c: P| ((a.0 - 2.0 * b.0 + c.0).powi(2) + (a.1 - 2.0 * b.1 + c.1).powi(2)).sqrt();
    let bound = 0.25 * d(p0, p1, p2).max(d(p1, p2, p3));
    segs_for(bound)
}

fn quad_segments(p0: P, p1: P, p2: P) -> usize {
    let d = ((p0.0 - 2.0 * p1.0 + p2.0).powi(2) + (p0.1 - 2.0 * p1.1 + p2.1).powi(2)).sqrt();
    segs_for(0.5 * d)
}

fn segs_for(bound: f64) -> usize {
    if bound <= MIN_ARC_SAGITTA_EM {
        return 1;
    }
    ((bound / MIN_ARC_SAGITTA_EM).sqrt().ceil() as usize).clamp(1, SEGMENT_CAP)
}

fn cubic(out: &mut Vec<P>, p0: P, p1: P, p2: P, p3: P) -> Result<(), PathError> {
    let n = cubic_segments(p0, p1, p2, p3);
    if n == SEGMENT_CAP && bound_exceeds(p0, p1, p2, p3) {
        return err(format!(
            "cubic needs more than {SEGMENT_CAP} segments to hold the flattening \
             tolerance; its control points are degenerate"
        ));
    }
    for i in 1..=n {
        let t = i as f64 / n as f64;
        out.push(cubic_at(p0, p1, p2, p3, t));
    }
    Ok(())
}

fn bound_exceeds(p0: P, p1: P, p2: P, p3: P) -> bool {
    let d = |a: P, b: P, c: P| ((a.0 - 2.0 * b.0 + c.0).powi(2) + (a.1 - 2.0 * b.1 + c.1).powi(2)).sqrt();
    let bound = 0.25 * d(p0, p1, p2).max(d(p1, p2, p3));
    (bound / MIN_ARC_SAGITTA_EM).sqrt() > SEGMENT_CAP as f64
}

fn quad(out: &mut Vec<P>, p0: P, p1: P, p2: P) -> Result<(), PathError> {
    let n = quad_segments(p0, p1, p2);
    for i in 1..=n {
        let t = i as f64 / n as f64;
        let u = 1.0 - t;
        out.push((
            u * u * p0.0 + 2.0 * u * t * p1.0 + t * t * p2.0,
            u * u * p0.1 + 2.0 * u * t * p1.1 + t * t * p2.1,
        ));
    }
    Ok(())
}

fn cubic_at(p0: P, p1: P, p2: P, p3: P, t: f64) -> P {
    let u = 1.0 - t;
    let (a, b) = (u * u * u, 3.0 * u * u * t);
    let (c, d) = (3.0 * u * t * t, t * t * t);
    (
        a * p0.0 + b * p1.0 + c * p2.0 + d * p3.0,
        a * p0.1 + b * p1.1 + c * p2.1 + d * p3.1,
    )
}

/// SVG's endpoint-to-centre arc conversion, then flattening.
///
/// This is the F.6.5 algorithm from the SVG 1.1 specification, including both
/// of its corrections for radii too small to span the chord — without those,
/// a degenerate arc silently becomes a straight line, which is how a ring
/// turns into a rule.
#[allow(clippy::too_many_arguments)]
fn arc(
    out: &mut Vec<P>,
    from: P,
    rx: f64,
    ry: f64,
    rot_deg: f64,
    large: bool,
    sweep: bool,
    to: P,
) -> Result<(), PathError> {
    if rx == 0.0 || ry == 0.0 {
        // The spec says treat this as a straight line.
        out.push(to);
        return Ok(());
    }
    let (rx, ry) = (rx.abs(), ry.abs());
    let phi = rot_deg.to_radians();
    let (cos_phi, sin_phi) = (phi.cos(), phi.sin());
    let dx2 = (from.0 - to.0) / 2.0;
    let dy2 = (from.1 - to.1) / 2.0;
    let x1p = cos_phi * dx2 + sin_phi * dy2;
    let y1p = -sin_phi * dx2 + cos_phi * dy2;

    // Scale the radii up if they are too small to reach the endpoints.
    let lambda = (x1p * x1p) / (rx * rx) + (y1p * y1p) / (ry * ry);
    let (rx, ry) = if lambda > 1.0 {
        let s = lambda.sqrt();
        (rx * s, ry * s)
    } else {
        (rx, ry)
    };

    let num = rx * rx * ry * ry - rx * rx * y1p * y1p - ry * ry * x1p * x1p;
    let den = rx * rx * y1p * y1p + ry * ry * x1p * x1p;
    let mut coef = if den <= 0.0 { 0.0 } else { (num / den).max(0.0).sqrt() };
    if large == sweep {
        coef = -coef;
    }
    let cxp = coef * (rx * y1p / ry);
    let cyp = coef * -(ry * x1p / rx);
    let cx = cos_phi * cxp - sin_phi * cyp + (from.0 + to.0) / 2.0;
    let cy = sin_phi * cxp + cos_phi * cyp + (from.1 + to.1) / 2.0;

    let angle = |ux: f64, uy: f64, vx: f64, vy: f64| -> f64 {
        let dot = ux * vx + uy * vy;
        let len = (ux * ux + uy * uy).sqrt() * (vx * vx + vy * vy).sqrt();
        let c = (dot / len).clamp(-1.0, 1.0);
        let a = c.acos();
        if ux * vy - uy * vx < 0.0 {
            -a
        } else {
            a
        }
    };
    let ux = (x1p - cxp) / rx;
    let uy = (y1p - cyp) / ry;
    let vx = (-x1p - cxp) / rx;
    let vy = (-y1p - cyp) / ry;
    let theta = angle(1.0, 0.0, ux, uy);
    let mut delta = angle(ux, uy, vx, vy);
    if !sweep && delta > 0.0 {
        delta -= 2.0 * std::f64::consts::PI;
    } else if sweep && delta < 0.0 {
        delta += 2.0 * std::f64::consts::PI;
    }

    // Segments from the arc's own sagitta, so a shallow arc gets few and a
    // half-circle gets many, at the same tolerance.
    let sweep_rad = delta.abs();
    let r_eff = rx.max(ry);
    let n = {
        let per = 2.0 * (1.0 - MIN_ARC_SAGITTA_EM / r_eff).clamp(-1.0, 1.0).acos();
        if per <= 0.0 {
            SEGMENT_CAP
        } else {
            ((sweep_rad / per).ceil() as usize).clamp(1, SEGMENT_CAP)
        }
    };
    // The parameter runs from `theta`, not from zero: `theta` is where on the
    // ellipse the arc's own start point sits, and the endpoint-to-centre
    // conversion above put the centre there. Starting the parameter at zero
    // instead draws a DIFFERENT arc through the same two endpoints — the
    // mirror image — which is a bug that still produces a plausible curve.
    for i in 1..=n {
        let t = i as f64 / n as f64;
        let a = theta + delta * t;
        let (ca, sa) = (a.cos(), a.sin());
        let px = cos_phi * rx * ca - sin_phi * ry * sa + cx;
        let py = sin_phi * rx * ca + cos_phi * ry * sa + cy;
        out.push((px, py));
    }
    Ok(())
}

// --- the lexer --------------------------------------------------------------

/// A hand-rolled number lexer, because `str::parse` cannot do this.
///
/// SVG allows a number's sign and its decimal point to act as separators, so
/// `10-20` is two numbers and `1.5.5` is `1.5` then `.5`. Splitting on
/// whitespace and commas — the obvious approach, and what most hand-rolled
/// parsers do — silently mis-reads both. Upstream's tables contain `10-20`
/// and `.5` forms, so this is not a hypothetical.
struct Lexer<'a> {
    src: &'a [u8],
    pos: usize,
}

impl<'a> Lexer<'a> {
    fn new(s: &'a str) -> Self {
        Lexer {
            src: s.as_bytes(),
            pos: 0,
        }
    }

    fn skip_separators(&mut self) {
        while self.pos < self.src.len()
            && (self.src[self.pos].is_ascii_whitespace() || self.src[self.pos] == b',')
        {
            self.pos += 1;
        }
    }

    fn peek_cmd(&mut self) -> Option<char> {
        self.skip_separators();
        let c = *self.src.get(self.pos)? as char;
        if c.is_ascii_alphabetic() {
            Some(c)
        } else {
            None
        }
    }

    fn take_cmd(&mut self) -> char {
        let c = self.src[self.pos] as char;
        self.pos += 1;
        c
    }

    fn peek_is_number(&mut self) -> bool {
        self.skip_separators();
        match self.src.get(self.pos) {
            Some(c) => c.is_ascii_digit() || matches!(c, b'+' | b'-' | b'.'),
            None => false,
        }
    }

    fn number(&mut self) -> Result<f64, PathError> {
        self.skip_separators();
        let start = self.pos;
        if matches!(self.src.get(self.pos), Some(b'+') | Some(b'-')) {
            self.pos += 1;
        }
        let mut digits = 0;
        while matches!(self.src.get(self.pos), Some(c) if c.is_ascii_digit()) {
            self.pos += 1;
            digits += 1;
        }
        if self.src.get(self.pos) == Some(&b'.') {
            self.pos += 1;
            while matches!(self.src.get(self.pos), Some(c) if c.is_ascii_digit()) {
                self.pos += 1;
                digits += 1;
            }
        }
        if digits == 0 {
            return err(format!(
                "expected a number at byte {start} of `{}`",
                String::from_utf8_lossy(&self.src[start..(start + 12).min(self.src.len())])
            ));
        }
        // An exponent only counts as one if the digits follow it, so that the
        // `e` in a malformed token does not swallow the next command.
        if matches!(self.src.get(self.pos), Some(b'e') | Some(b'E')) {
            let save = self.pos;
            self.pos += 1;
            if matches!(self.src.get(self.pos), Some(b'+') | Some(b'-')) {
                self.pos += 1;
            }
            if matches!(self.src.get(self.pos), Some(c) if c.is_ascii_digit()) {
                while matches!(self.src.get(self.pos), Some(c) if c.is_ascii_digit()) {
                    self.pos += 1;
                }
            } else {
                self.pos = save;
            }
        }
        let text = std::str::from_utf8(&self.src[start..self.pos])
            .map_err(|_| PathError("path is not utf-8".into()))?;
        let v: f64 = text
            .parse()
            .map_err(|_| PathError(format!("`{text}` is not a number")))?;
        // `parse` accepts `1e999` as infinity, which passes the grammar and
        // then poisons every box comparison downstream.
        if !v.is_finite() {
            return err(format!("`{text}` is not finite"));
        }
        Ok(v)
    }

    /// An arc flag: exactly one `0` or `1`, per the SVG grammar.
    fn flag(&mut self) -> Result<bool, PathError> {
        self.skip_separators();
        match self.src.get(self.pos) {
            Some(b'0') => {
                self.pos += 1;
                Ok(false)
            }
            Some(b'1') => {
                self.pos += 1;
                Ok(true)
            }
            _ => err("an arc flag must be 0 or 1"),
        }
    }

    fn point(&mut self, rel: bool, origin: P) -> Result<P, PathError> {
        let x = self.number()?;
        let y = self.number()?;
        Ok(if rel { (origin.0 + x, origin.1 + y) } else { (x, y) })
    }

    /// The six operands after an `A`, for an implicit repeat.
    fn arc_tail(&mut self, rel: bool, origin: P) -> Result<(f64, f64, f64, bool, bool, P), PathError> {
        let rx = self.number()?;
        let ry = self.number()?;
        let rot = self.number()?;
        let large = self.flag()?;
        let sweep = self.flag()?;
        let p = self.point(rel, origin)?;
        Ok((rx, ry, rot, large, sweep, p))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pts(d: &str) -> Vec<Vec<P>> {
        parse_path(d).expect("parses").into_iter().map(|r| r.points).collect()
    }

    #[test]
    fn a_sign_is_a_separator() {
        // The case whitespace-splitting gets wrong, and which the tables use.
        let a = pts("M10 0 L20-5");
        let b = pts("M10,0 L20,-5");
        assert_eq!(a, b);
        assert_eq!(a[0], vec![(10.0, 0.0), (20.0, -5.0)]);
    }

    #[test]
    fn a_leading_dot_is_a_number() {
        // `.5.5` is `0.5` then `0.5`: a dot separates numbers as well as
        // introducing a fraction. Splitting on whitespace reads it as one
        // token and gives up, and upstream writes this form.
        assert_eq!(pts("M.5.5 L1 1")[0][1], (1.0, 1.0));
    }

    #[test]
    fn repeated_pairs_after_a_move_are_lines() {
        // Not more moves. Getting this wrong turns a pentagon into a scribble.
        let r = pts("M0 0 10 0 10 10");
        assert_eq!(r.len(), 1);
        assert_eq!(r[0], vec![(0.0, 0.0), (10.0, 0.0), (10.0, 10.0)]);
    }

    #[test]
    fn a_close_reports_closed_and_repeats_the_start() {
        let runs = parse_path("M0 0 L10 0 L10 10 Z").expect("parses");
        assert!(runs[0].closed);
        assert_eq!(runs[0].points.last(), Some(&(0.0, 0.0)));
    }

    #[test]
    fn a_second_move_splits_the_runs() {
        assert_eq!(pts("M0 0 L10 0 M20 0 L30 0").len(), 2);
    }

    #[test]
    fn h_and_v_take_one_operand() {
        assert_eq!(pts("M1 2 H5 V7")[0], vec![(1.0, 2.0), (5.0, 2.0), (5.0, 7.0)]);
    }

    #[test]
    fn a_cubic_is_flattened_within_tolerance() {
        // The anchor's shank-to-fluke curve, sampled and checked against the
        // real curve rather than against a point count.
        let d = "M0 0 C0 50 100 50 100 0";
        let r = &parse_path(d).expect("parses")[0];
        for (i, p) in r.points.iter().enumerate() {
            let t = i as f64 / (r.points.len() - 1) as f64;
            let u = 1.0 - t;
            let (bx, by) = (
                u * u * u * 0.0 + 3.0 * u * u * t * 0.0 + 3.0 * u * t * t * 100.0 + t * t * t * 100.0,
                u * u * u * 0.0 + 3.0 * u * u * t * 50.0 + 3.0 * u * t * t * 50.0 + t * t * t * 0.0,
            );
            assert!((p.0 - bx).hypot(p.1 - by) <= MIN_ARC_SAGITTA_EM + 1e-9, "{p:?}");
        }
    }

    #[test]
    fn a_straight_cubic_needs_one_segment() {
        let r = &parse_path("M0 0 C33 66 66 99 99 99").expect("parses")[0];
        assert_eq!(r.points.len(), 2);
    }

    #[test]
    fn a_smooth_cubic_mirrors_the_previous_control() {
        // `S` with no preceding cubic coincides its first control with the
        // current point, which degenerates to a quadratic rather than failing.
        assert!(parse_path("M0 0 S50 50 100 0").is_ok());
        // And with a preceding cubic it reflects it, so the curve is C1.
        let a = &parse_path("M0 0 C0 10 20 10 20 0 S40 -10 40 0").expect("parses")[0];
        assert!(a.points.len() > 2);
    }

    #[test]
    fn a_quadratic_and_a_smooth_quadratic_parse() {
        assert!(parse_path("M0 0 Q50 50 100 0").is_ok());
        assert!(parse_path("M0 0 Q50 50 100 0 T200 0").is_ok());
    }

    #[test]
    fn an_arc_bulges_the_sweep_way() {
        // A semicircle from (0,0) to (100,0) with rx=ry=50. Sweep 1 means
        // "positive angle direction", and in SVG's y-DOWN space that is
        // towards negative y — so it bulges UP the screen. Getting this
        // backwards yields a curve that is still smooth and still joins the
        // right two endpoints, which is why it is pinned by a test and not by
        // a comment.
        //
        // Checked against the true circle rather than against the apex: the
        // flattened vertices sit on the circle but not on the apex, so
        // asserting "some vertex is at y = -50" would be asserting an accident
        // of the segment count. What must hold is that every vertex is ON the
        // circle, that they are all on the swept side, and that the extreme
        // reaches the apex to within the flattening tolerance.
        let on_circle = |p: P| ((p.0 - 50.0).powi(2) + p.1.powi(2)).sqrt() - 50.0;
        let apex = |ps: &[P], want_min: bool| {
            let e = if want_min {
                ps.iter().map(|p| p.1).fold(f64::MAX, f64::min)
            } else {
                ps.iter().map(|p| p.1).fold(f64::MIN, f64::max)
            };
            e
        };

        let s1 = &parse_path("M0 0 A50 50 0 0 1 100 0").expect("parses")[0];
        assert!(
            s1.points.iter().all(|p| on_circle(*p).abs() < 1e-6),
            "{:?}",
            s1.points
        );
        assert!(s1.points.iter().all(|p| p.1 <= 1e-9), "sweep 1 is above");
        assert!(
            (apex(&s1.points, true) + 50.0).abs() < MIN_ARC_SAGITTA_EM,
            "and reaches the apex to within the tolerance: {:?}",
            s1.points
        );

        let s0 = &parse_path("M0 0 A50 50 0 0 0 100 0").expect("parses")[0];
        assert!(
            s0.points.iter().all(|p| on_circle(*p).abs() < 1e-6),
            "{:?}",
            s0.points
        );
        assert!(s0.points.iter().all(|p| p.1 >= -1e-9), "sweep 0 is below");
        assert!(
            (apex(&s0.points, false) - 50.0).abs() < MIN_ARC_SAGITTA_EM,
            "and reaches the apex too: {:?}",
            s0.points
        );

        // Both must start and end where they were asked to.
        assert_eq!(s1.points.first(), Some(&(0.0, 0.0)));
        assert!(s1.points.last().is_some_and(|p| (p.0 - 100.0).abs() < 1e-9));
    }

    #[test]
    fn an_arc_too_small_for_its_chord_is_corrected_not_dropped() {
        // rx=ry=10 cannot span a 100-wide chord. Without the spec's radius
        // correction this collapses to a straight line and a ring becomes a
        // rule; the correction scales the radii up and it stays a curve.
        let r = &parse_path("M0 0 A10 10 0 0 1 100 0").expect("parses")[0];
        assert!(r.points.iter().any(|p| p.1.abs() > 20.0), "{r:?}");
    }

    #[test]
    fn a_zero_radius_arc_is_a_line() {
        let r = &parse_path("M0 0 L10 0 A0 0 0 0 1 20 0").expect("parses")[0];
        assert_eq!(r.points.last(), Some(&(20.0, 0.0)));
    }

    #[test]
    fn a_relative_command_after_close_measures_from_the_start() {
        // SVG 1.1: `Z` ends the subpath and returns the current point to its
        // first point, and a following command opens a NEW subpath from
        // there. So this is two runs and the second starts at (10,10) again.
        let runs = parse_path("M10 10 L20 20 Z l5 0").expect("parses");
        assert_eq!(runs.len(), 2, "{runs:?}");
        assert!(runs[0].closed);
        assert_eq!(runs[1].points, vec![(10.0, 10.0), (15.0, 10.0)]);
    }

    #[test]
    fn an_open_two_point_run_encloses_nothing() {
        // The infantry saltire. Filling it draws nothing, which is why the
        // fill decision is made from the geometry rather than the flag; the
        // threshold that turns this into a stroke lives in `generate`.
        let runs = parse_path("M25 50 L175 150 M25,150 175,50").expect("parses");
        assert_eq!(runs.len(), 2);
        for r in &runs {
            assert_eq!(r.enclosed_area(), 0.0, "{r:?}");
        }
    }

    #[test]
    fn a_closed_loop_encloses_its_area() {
        // A right triangle of legs 50, so 1250 — NOT 2500, which is what the
        // same points give if the closing edge is counted twice.
        let r = &parse_path("M0 0 L50 0 L50 50 Z").expect("parses")[0];
        assert!((r.enclosed_area() - 1250.0).abs() < 1e-9, "{}", r.enclosed_area());
        assert_eq!(shoelace(&[(0.0, 0.0), (10.0, 0.0), (10.0, 10.0)]), 50.0);
        // An open ring closes itself, so the implicit edge counts once.
        assert_eq!(
            shoelace(&[(0.0, 0.0), (10.0, 0.0), (10.0, 10.0)]),
            shoelace(&[(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 0.0)]),
            "repeating the first point must not double the area"
        );
        assert_eq!(
            shoelace(&[(0.0, 0.0), (10.0, 0.0)]),
            0.0,
            "a line encloses nothing"
        );
    }

    #[test]
    fn an_unknown_command_is_refused() {
        assert!(parse_path("M0 0 K10 10").is_err());
    }

    #[test]
    fn a_truncated_command_is_refused() {
        assert!(parse_path("M0 0 L10").is_err());
        assert!(parse_path("M0 0 C1 2 3").is_err());
    }

    #[test]
    fn a_leading_relative_move_resolves_against_the_origin() {
        // Upstream writes `m 80,100` for a move to (80,100). SVG resolves it
        // against the origin because there is no current point yet, so
        // refusing it would refuse most of the sea vocabulary.
        let r = &parse_path("m 80 100 20 20 20 -20 z").expect("parses")[0];
        assert_eq!(r.points[0], (80.0, 100.0));
        assert_eq!(r.points[1], (100.0, 120.0));
        assert!(r.closed);
    }

    #[test]
    fn a_path_with_no_drawable_run_is_refused() {
        assert!(parse_path("M10 10").is_err());
        assert!(parse_path("").is_err());
    }

    #[test]
    fn a_non_finite_number_is_refused() {
        // `1e999` parses to infinity in Rust and passes a naive split.
        assert!(parse_path("M0 0 L1e999 0").is_err());
    }
}
