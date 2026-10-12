//! The halo, from both ends: WGSL parity, then what a GPU-less surface can
//! honestly do with it (U7).
//!
//! Two questions, both answered with numbers rather than argument:
//!
//! 1. **Parity.** `p5-epaint --fx-selftest --fx-dump <png>` writes the
//!    WGSL's offscreen target. [`tfg::fx::rasterize`] evaluates the same
//!    formula on the CPU. Diffing the two says whether the CPU path is the
//!    same effect or merely a similar one.
//! 2. **Blend.** GPUI 0.2.2 has no per-element blend mode and no device
//!    access (measured in the crate's own source; see the handoff), so a
//!    GPUI surface cannot do the shader's `One/One`. The nearest thing it
//!    can do is paint the same buffer source-over. That is not the same
//!    operation — source-over attenuates what is behind it — so this
//!    measures how much, on a real map frame.
//!
//! Run: `cargo run --release --example halo_seam`
//! Env: `FX_DUMP` (shader readback, default `/tmp/opencode/fx-shader.png`),
//! `MAP_FRAME` (backdrop, default `/tmp/opencode/mapframes/frame_000.png`),
//! `HALO_STRENGTH` (default 0.55, the console's selected-marker value).

use image::{Rgba, RgbaImage};
use tfg::fx::Halo;

/// The GPU selftest draws into a 256x256 target with the quad's half-extent
/// at HALO_R pixels, so the halo occupies a 2*HALO_R box about the centre.
const DUMP_SIZE: usize = 256;

fn main() {
    let dump_path =
        std::env::var("FX_DUMP").unwrap_or_else(|_| "/tmp/opencode/fx-shader.png".to_string());
    let frame_path = std::env::var("MAP_FRAME")
        .unwrap_or_else(|_| "/tmp/opencode/mapframes/frame_000.png".to_string());
    let strength: f32 = std::env::var("HALO_STRENGTH")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0.55);

    let tint = Halo::Signal.rgb();
    let side = (tfg::fx::HALO_R * 2.0) as usize; // 44

    // ---- 1. Parity: CPU raster vs the WGSL's own readback ----------------
    let dump = image::open(&dump_path)
        .unwrap_or_else(|e| panic!("shader dump {dump_path}: {e}"))
        .to_rgba8();
    assert_eq!(dump.dimensions(), (DUMP_SIZE as u32, DUMP_SIZE as u32));
    let cpu = tfg::fx::rasterize(side, tint, 1.0);

    let origin = DUMP_SIZE / 2 - side / 2; // 106
    let mut max_delta = 0i32;
    let mut differing = 0usize;
    for y in 0..side {
        for x in 0..side {
            let g = dump.get_pixel((origin + x) as u32, (origin + y) as u32).0;
            let c = &cpu[(y * side + x) * 4..(y * side + x) * 4 + 4];
            let mut differs = false;
            for ch in 0..3 {
                let d = (g[ch] as i32 - c[ch] as i32).abs();
                max_delta = max_delta.max(d);
                if d != 0 {
                    differs = true;
                }
            }
            if differs {
                differing += 1;
            }
        }
    }
    println!(
        "parity: CPU raster vs WGSL readback over {side}x{side} samples — max channel delta {max_delta}, \
         pixels differing {differing}/{}",
        side * side
    );
    // The ring radii the console pairs with these halos, printed here so the
    // GPUI side can quote the real number instead of a stale literal.
    println!(
        "rings: symbol footprint radius {:.2}px -> selected {:.2}, following {:.2}, old-data {:.2}",
        tfg::map_render::symbol_footprint_radius_px(),
        tfg::map_render::symbol_footprint_radius_px() * 1.5,
        tfg::map_render::symbol_footprint_radius_px() * 1.75,
        tfg::map_render::symbol_footprint_radius_px() * 2.0,
    );

    // ---- 2. Blend: additive vs source-over, on a real map frame ----------
    let mut frame = image::open(&frame_path)
        .unwrap_or_else(|e| panic!("map frame {frame_path}: {e}"))
        .to_rgba8();
    let (fw, fh) = frame.dimensions();
    // Centre the halo; a marker rarely sits at the exact edge and that is
    // not what is being measured.
    let cx = fw as i64 / 2;
    let cy = fh as i64 / 2;

    let mut additive = frame.clone();
    let mut overlay = frame.clone();
    let mut darkened = 0usize;
    let mut worst_darkening = 0i32;
    let mut max_blend_gap = 0i32;
    for y in 0..side {
        for x in 0..side {
            let px = cx - side as i64 / 2 + x as i64;
            let py = cy - side as i64 / 2 + y as i64;
            let c = &cpu[(y * side + x) * 4..(y * side + x) * 4 + 4];
            let src = [c[0] as f32, c[1] as f32, c[2] as f32, c[3] as f32];
            // The raster is at strength 1; scale to the marker's strength.
            let s = [src[0] * strength, src[1] * strength, src[2] * strength, src[3] * strength];
            let dst = overlay.get_pixel(px as u32, py as u32).0;

            // Additive: dst + src, clamped — exactly the shader's One/One.
            let add = [
                (dst[0] as f32 + s[0]).min(255.0) as u8,
                (dst[1] as f32 + s[1]).min(255.0) as u8,
                (dst[2] as f32 + s[2]).min(255.0) as u8,
                255,
            ];
            additive.put_pixel(px as u32, py as u32, Rgba(add));

            // Source-over with premultiplied colour: the only blend a GPUI
            // element offers.
            let a = s[3] / 255.0;
            let over = [
                (s[0] + dst[0] as f32 * (1.0 - a)) as u8,
                (s[1] + dst[1] as f32 * (1.0 - a)) as u8,
                (s[2] + dst[2] as f32 * (1.0 - a)) as u8,
                255,
            ];
            overlay.put_pixel(px as u32, py as u32, Rgba(over));

            let lum = |p: [u8; 4]| (p[0] as f32 * 0.299 + p[1] as f32 * 0.587 + p[2] as f32 * 0.114);
            let back = lum(dst);
            let lit = lum(over);
            if lit < back {
                darkened += 1;
                worst_darkening = worst_darkening.max((back - lit) as i32);
            }
            for ch in 0..3 {
                max_blend_gap = max_blend_gap.max((add[ch] as i32 - over[ch] as i32).abs());
            }
        }
    }

    let total = (side * side) as f32;
    println!(
        "blend: strength {strength} over {}x{} of {frame_path}",
        fw, fh
    );
    println!(
        "blend: source-over darkens {darkened}/{side}x{side} pixels (worst {worst_darkening}/255 luminance); \
         max additive-vs-source-over channel gap {max_blend_gap}"
    );
    println!(
        "blend: {:.1}% of the halo's footprint is attenuated where the tint is darker than the map",
        100.0 * darkened as f32 / total
    );

    let stem = "/tmp/opencode/halo";
    additive.save(format!("{stem}-additive.png")).unwrap();
    overlay.save(format!("{stem}-sourceover.png")).unwrap();
    println!("wrote {stem}-additive.png and {stem}-sourceover.png");

    // A panel for the eye: frame, additive, source-over, side by side.
    let mut panel = RgbaImage::new(fw * 3, fh);
    for y in 0..fh {
        for x in 0..fw {
            panel.put_pixel(x, y, *frame.get_pixel(x, y));
            panel.put_pixel(fw + x, y, *additive.get_pixel(x, y));
            panel.put_pixel(fw * 2 + x, y, *overlay.get_pixel(x, y));
        }
    }
    panel.save("/tmp/opencode/halo-panel.png").unwrap();
    println!("wrote /tmp/opencode/halo-panel.png (frame | additive | source-over)");
}
