//! The halo's arithmetic, without wgpu.
//!
//! Split out of [`super`] so the falloff can be evaluated, tested and
//! rasterized where no device exists — a CPU rasterizer for the surfaces
//! GPUI can draw, and an oracle the WGSL can be checked against. Both
//! renderers read these definitions, so they cannot drift.
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
