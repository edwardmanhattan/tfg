//! The console's palette and icons at the toolkit boundary.
//!
//! The kernel (`view`) speaks 0..255 sRGB and no toolkit type at all — that
//! is the seam the shell migration turns on, and it is why the palette is
//! testable with no display. This module is the ONE place that converts it
//! to the shell's GPUI colour, so every surface that draws the console's ink
//! draws it from the same numbers.
//!
//! It is a module and not a function inside the shell because the pickers
//! are modules too, and a shared helper that lives in a 38,000-line file is
//! a helper the second module cannot reach.

use crate::view::Rgba;
use gpui::{IntoElement, Styled, px};

/// One `view::Rgba` as a GPUI colour.
///
/// `gpui::rgba` is raw 0..1 with no gamma applied, so this is the exact same
/// convention the design tokens use and the values pass through untouched.
/// Going by way of `Hsla` instead applies a conversion the tokens never
/// asked for and shifts every hue.
pub fn rgba(c: Rgba) -> gpui::Rgba {
    let [r, g, b, _] = c.as_f32();
    gpui::Rgba { r, g, b, a: c.alpha_f32() }
}

/// One of the console's drawn icons, as an SVG at a size.
///
/// The icons are COMPILED IN, not read from disk: the console is air-gapped
/// and the one asset source in the binary is the only one it needs.
pub fn icon(path: &'static str, w: f32, h: f32, ink: Rgba) -> impl IntoElement {
    gpui::svg()
        .path(path)
        .w(px(w))
        .h(px(h))
        .text_color(rgba(ink))
}
