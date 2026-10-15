//! The halo's arithmetic, without wgpu.
//!
//! Split out of `fx` so the falloff can be evaluated, tested and
//! rasterized where no device exists — a CPU rasterizer for the surfaces
//! GPUI can draw, and an oracle the WGSL can be checked against. Both
//! renderers read these definitions, so they cannot drift.
//!
//! It lives at the crate root rather than under `fx` because the GPUI
//! shell paints sprites from it while `fx` is egui painters: keeping it
//! under `fx` would weld the shell's build to eframe. `fx` re-exports
//! the names so the console's paths do not move.
//!
//! The shader's contract, in one line each:
//!
//! - `d` is the distance from the halo centre in units of `HALO_R`: 0 at
//!   the centre, 1 at the inscribed circle of the quad, `sqrt(2)` at its
//!   corners.
//! - The contribution is `tint_rgb * strength * fade(d)`, added to the
//!   target. Nothing outside `d >= 1`.
//! - `fade(0) == 1` exactly, and flat until `0.35`, so the centre reads as
//!   the pure tint — which is the number the GPU selftest asserts.

/// Radius in logical points. Sized to read as a halo rather than a glow
/// field: it clears the 8px marker and its 2px ring, and stops well before
/// it starts washing the trails underneath.
pub const HALO_R: f32 = 22.0;

/// The three marker states the epaint rings already carry. The halo only
/// reinforces a channel that exists.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Halo {
    /// Selected: radar cyan.
    Signal,
    /// Old data: the stale amber from DESIGN.md.
    Stale,
    /// Following a fix's move: green, fading as the map forgets it.
    Embark,
}

impl Halo {
    /// Tint as 0..1 floats, matching the shader's uniform.
    pub fn rgb(self) -> [f32; 3] {
        match self {
            Halo::Signal => [0.133, 0.827, 0.933],
            Halo::Stale => [0.965, 0.773, 0.420],
            Halo::Embark => [0.290, 0.871, 0.502],
        }
    }
}

/// `smoothstep` as the WGSL defines it, so the CPU and GPU agree at the
/// ends and in the middle of the ramp.
fn smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// The halo's radial fade at `d` (distance in units of [`HALO_R`]).
/// Squared, so the falloff is flat near the marker and steep at the edge.
pub fn fade(d: f32) -> f32 {
    let a = 1.0 - smoothstep(0.35, 1.0, d);
    a * a
}

/// Rasterize one halo into a square `size` x `size` RGBA buffer.
///
/// The colour is the ADDITIVE contribution (`rgb = tint * strength *
/// fade`), and alpha carries the same weight — so the same buffer serves
/// either blend: additive reads the colour, source-over reads the alpha
/// and treats the colour as premultiplied. Which of the two a surface can
/// do is the surface's problem, not this function's.
///
/// `size` is `2 * HALO_R * pixels_per_point`, rounded; the sampled `d`
/// uses pixel centres, matching a GPU rasterizer's sample positions.
pub fn rasterize(size: usize, tint: [f32; 3], strength: f32) -> Vec<u8> {
    let mut out = vec![0u8; size * size * 4];
    for y in 0..size {
        for x in 0..size {
            // Corner space: -1 at the first pixel's left edge, +1 at the
            // last pixel's right edge, sampled at the pixel centre. That is
            // the same `uv` the vertex shader interpolates, and because the
            // quad's half-extent is HALO_R on both axes, `length(uv)` is
            // already the distance in units of HALO_R.
            let u = ((x as f32 + 0.5) / size as f32) * 2.0 - 1.0;
            let v = ((y as f32 + 0.5) / size as f32) * 2.0 - 1.0;
            let d = (u * u + v * v).sqrt();
            let a = fade(d) * strength;
            let i = (y * size + x) * 4;
            let to8 = |c: f32| (c.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
            out[i] = to8(tint[0] * a);
            out[i + 1] = to8(tint[1] * a);
            out[i + 2] = to8(tint[2] * a);
            out[i + 3] = to8(a);
        }
    }
    out
}

#[cfg(test)]
mod halo_tests {
    use super::*;

    /// The shader's own contract, asserted on the CPU side: the centre is
    /// exactly full strength, and nothing at or beyond the inscribed circle
    /// contributes. The GPU selftest (`p5-epaint --fx-selftest`) asserts the
    /// same two facts plus the peak; this is the other half of the parity
    /// claim, and it runs without a device.
    #[test]
    fn fade_is_one_at_the_centre_and_zero_at_the_rim() {
        assert_eq!(fade(0.0), 1.0);
        for d in [1.0, 1.2, 1.414, 3.0] {
            assert_eq!(fade(d), 0.0, "fade({d}) must be exactly zero");
        }
    }

    #[test]
    fn fade_is_flat_inside_the_marker_then_monotonic() {
        // Flat to 0.35 — that is what makes the centre read as pure tint
        // rather than a curve, and it is why the peak is the tint sum.
        assert_eq!(fade(0.34), 1.0);
        let mut prev = fade(0.35);
        for i in 36..=100 {
            let d = i as f32 / 100.0;
            let a = fade(d);
            assert!(a <= prev, "fade must not rise: {d} gave {a} after {prev}");
            prev = a;
        }
    }

    /// The number the GPU selftest asserts, reproduced on the CPU: cyan at
    /// strength 1 has channel sum 483, measured 2026-10-11 on the real
    /// driver. 0.133+0.827+0.933 = 1.893, x255 = 482.7 -> 483.
    #[test]
    fn centre_pixel_matches_the_shader_selftest_peak() {
        let px = rasterize(44, Halo::Signal.rgb(), 1.0);
        let c = ((22 * 44) + 22) * 4;
        let sum = px[c] as u32 + px[c + 1] as u32 + px[c + 2] as u32;
        assert_eq!(sum, 483, "centre = {:?}", &px[c..c + 4]);
    }

    /// Outside the inscribed circle the contribution is zero on every
    /// channel, including alpha — so an input frame is what comes back out.
    #[test]
    fn corners_contribute_nothing() {
        let px = rasterize(44, Halo::Stale.rgb(), 1.0);
        for (x, y) in [(0usize, 0usize), (43, 0), (0, 43), (43, 43)] {
            let i = (y * 44 + x) * 4;
            assert_eq!(px[i..i + 4], [0, 0, 0, 0], "corner {x},{y} must be clear");
        }
    }

    /// Alpha carries the same weight as the colour, which is what lets a
    /// source-over surface composite the same buffer.
    #[test]
    fn alpha_tracks_the_additive_weight() {
        let px = rasterize(44, Halo::Embark.rgb(), 0.5);
        let c = ((22 * 44) + 22) * 4;
        // 0.5 strength at full fade -> alpha 128, colour at half tint.
        assert_eq!(px[c + 3], 128);
        assert_eq!(px[c + 1], (0.871 * 0.5 * 255.0 + 0.5) as u8);
    }
}
