//! Turning a hull's picture on its course.
//!
//! A hull's photograph is drawn nose to tail: the asset's bow points to the
//! image's +X edge, so an unturned picture points EAST, and a hull on a
//! course of 090 is drawn exactly as the asset is. A course of 000 wants the
//! bow pointing up the screen, and a course of 045 wants it half way — so
//! the picture is TURNED.
//!
//! GPUI 0.2.2 cannot do it: `Window::paint_image` takes an axis-aligned
//! box, and the only transformation in the toolkit belongs to `svg`, which
//! paints a silhouette in one colour. So the turning happens here, on the
//! pixels, and it happens STRAIGHT INTO the box the picture is drawn in: the
//! console never holds a turned picture bigger than the box it goes into.
//!
//! The turning is a sampling, not a resample-and-then-rotate: each
//! destination pixel asks the source where it came from, so the picture is
//! antialiased by the sampling and is drawn at the size it is SEEN.

/// Turn a picture by `degrees` clockwise on the screen, straight into the
/// box it will be drawn in.
///
/// `kx` and `ky` are the picture's own scale: how many of the picture's
/// pixels cover one SCREEN pixel, horizontally and vertically. They are not
/// equal when the picture's proportions differ from the hull's published
/// ones, and the difference is the point: the map draws a hull at the size
/// the register published, so the picture is stretched to those proportions
/// BEFORE it is turned, and the turning has to know the stretch.
///
/// The four channels are carried through untouched, so the console's
/// pictures stay in the format they arrived in (BGRA, the sprite atlas's).
///
/// `None` on a byte count that does not match the picture: a torn picture
/// paints nothing rather than a torn one.
pub fn turn_to_size(
    src: &[u8],
    w: u32,
    h: u32,
    degrees: f32,
    out_w: u32,
    out_h: u32,
    kx: f32,
    ky: f32,
) -> Option<Vec<u8>> {
    if w == 0 || h == 0 || out_w == 0 || out_h == 0 {
        return None;
    }
    let src_len = (w as usize) * (h as usize) * 4;
    if src.len() < src_len {
        return None;
    }
    let r = degrees.to_radians();
    let (sin, cos) = (r.sin(), r.cos());
    let mut out = vec![0u8; (out_w as usize) * (out_h as usize) * 4];
    // The centre of the destination, and of the source: the turning is about
    // the picture's own centre, and the two boxes share it.
    let (ox, oy) = (out_w as f32 / 2.0, out_h as f32 / 2.0);
    let (cx, cy) = (w as f32 / 2.0, h as f32 / 2.0);
    for y in 0..out_h {
        for x in 0..out_w {
            // Where in the picture does this destination pixel come from?
            // The turn is undone first, and the stretch after it: the
            // destination is in screen pixels and the source is in the
            // picture's own.
            let dx = x as f32 + 0.5 - ox;
            let dy = y as f32 + 0.5 - oy;
            let ux = cos * dx + sin * dy;
            let uy = -sin * dx + cos * dy;
            let sx = ux * kx + cx - 0.5;
            let sy = uy * ky + cy - 0.5;
            let (x0, y0) = (sx.floor(), sy.floor());
            let (fx, fy) = (sx - x0, sy - y0);
            let (x0, y0) = (x0 as i32, y0 as i32);
            let mut acc = [0f32; 4];
            for (ox, oy, weight) in [
                (0i32, 0i32, (1.0 - fx) * (1.0 - fy)),
                (1, 0, fx * (1.0 - fy)),
                (0, 1, (1.0 - fx) * fy),
                (1, 1, fx * fy),
            ] {
                let (px, py) = (x0 + ox, y0 + oy);
                if px < 0 || py < 0 || px >= w as i32 || py >= h as i32 {
                    // Outside the picture: the corner the turn opened up.
                    // Transparent, so the water shows through it.
                    continue;
                }
                let i = (py as usize) * (w as usize) * 4 + (px as usize) * 4;
                for c in 0..4 {
                    acc[c] += src[i + c] as f32 * weight;
                }
            }
            let o = (y as usize) * (out_w as usize) * 4 + (x as usize) * 4;
            for c in 0..4 {
                out[o + c] = acc[c].round().clamp(0.0, 255.0) as u8;
            }
        }
    }
    Some(out)
}

/// The box a picture of `w`×`h` occupies when turned by `degrees`.
///
/// The bounding box of the turned rectangle: a picture turned by a quarter
/// keeps its own box, and one turned by half a right angle is as wide and as
/// tall as the two of them added.
pub fn turned_size(w: f32, h: f32, degrees: f32) -> (f32, f32) {
    let r = degrees.to_radians();
    let (sin, cos) = (r.sin().abs(), r.cos().abs());
    (w * cos + h * sin, w * sin + h * cos)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A picture of two pixels side by side: red on the left, blue on the
    /// right.
    fn two_pixels() -> (Vec<u8>, u32, u32) {
        (
            vec![
                255, 0, 0, 255, // red
                0, 0, 255, 255, // blue
            ],
            2,
            1,
        )
    }

    /// A QUARTER TURN PUTS THE LEFT PIXEL ON TOP. The picture's bow points to
    /// the +X edge, so a quarter turn clockwise puts what was pointing right
    /// pointing down — and what was on the left is now on the top.
    #[test]
    fn a_quarter_turn_puts_the_left_pixel_on_top() {
        let (src, w, h) = two_pixels();
        // One source pixel per screen pixel, so the picture is not scaled.
        let out = turn_to_size(&src, w, h, 90.0, 1, 2, 1.0, 1.0).expect("the picture turns");
        assert_eq!(out.len(), 1 * 2 * 4, "the box is the picture's own");
        let top = &out[0..4];
        let bottom = &out[4..8];
        assert!(
            top[0] > top[2] && bottom[2] > bottom[0],
            "red is on top and blue is under it: {top:?} {bottom:?}"
        );
    }

    /// THE BOX GROWS WITH THE TURN. A quarter turn of a long thin picture is
    /// as tall as it is wide, and the map has to know that before it draws:
    /// a turned picture is painted into the box it occupies, not into the
    /// box it started in.
    #[test]
    fn the_box_grows_with_the_turn() {
        assert_eq!(turned_size(100.0, 20.0, 0.0), (100.0, 20.0));
        let (w, h) = turned_size(100.0, 20.0, 90.0);
        assert!((w - 20.0).abs() < 0.01, "a quarter turn swaps the sides: {w}");
        assert!((h - 100.0).abs() < 0.01, "a quarter turn swaps the sides: {h}");
        let (w, h) = turned_size(100.0, 20.0, 45.0);
        assert!(
            (w - (100.0 + 20.0) * 0.707_106_8).abs() < 0.01,
            "half a right angle is the two of them added: {w}"
        );
        assert!((h - w).abs() < 0.01, "and a square: {w} {h}");
    }

    /// THE CORNER THE TURN OPENS UP IS TRANSPARENT. The water shows through
    /// it, and a picture with a solid corner would draw a wedge of sea on
    /// the map.
    #[test]
    fn the_corner_the_turn_opens_up_is_transparent() {
        let (src, w, h) = two_pixels();
        let out = turn_to_size(&src, w, h, 45.0, 4, 4, 1.0, 1.0).expect("the picture turns");
        let alphas: Vec<u8> = (0..16).map(|i| out[i * 4 + 3]).collect();
        assert!(
            alphas.iter().any(|a| *a == 0),
            "the opened corner is transparent: {alphas:?}"
        );
    }

    /// A TORN PICTURE PAINTS NOTHING. The byte count is the picture's
    /// integrity, and a short buffer is a fetch that went wrong.
    #[test]
    fn a_torn_picture_paints_nothing() {
        let (src, w, h) = two_pixels();
        assert!(
            turn_to_size(&src[..3], w, h, 90.0, 2, 2, 1.0, 1.0).is_none(),
            "a short buffer is refused"
        );
    }

    /// A TURN OF NOTHING IS THE PICTURE. The map draws a hull with no course
    /// on the asset's own axis, and the turning must not move it.
    #[test]
    fn a_turn_of_nothing_is_the_picture() {
        let (src, w, h) = two_pixels();
        let out = turn_to_size(&src, w, h, 0.0, 2, 1, 1.0, 1.0).expect("the picture turns");
        assert_eq!(out, src, "the picture is where it was");
    }
}
