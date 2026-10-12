# ADR-0019: The halo does not cross to GPUI — the tier is Low

## Status

Accepted. Applies to the GPUI shell under migration; the egui console keeps
ADR-0015's effect unchanged.

## Context

ADR-0015 put one shader in the client: an additive halo on the three marker
states, reached through `egui_wgpu::CallbackTrait`. The migration's U7 asks
what that seam becomes in GPUI. It was measured, not assumed, against pinned
`gpui` 0.2.2 on Linux:

- **No per-element blend mode.** The only blend state in the crate belongs to
  the surface (`blade_renderer.rs:158`, driven by `WindowBackgroundAppearance`).
  Elements composite source-over, always.
- **No device or queue access.** `Window` exposes `gpu_specs()` — the adapter's
  device type and name — and nothing else. `Element::paint` can only call
  `paint_quad`, `paint_path`, `paint_image`, `paint_glyph`.
- **`surface()` is macOS-only** (`SurfaceSource::Surface(CVPixelBuffer)`); the
  Linux arm matches nothing. There is no zero-copy external-texture path, which
  is what ADR-0002 already observed.
- **Atlas images are STRAIGHT alpha.** gpui's decoder swaps R for B and never
  premultiplies (`elements/img.rs`), so the compositor applies alpha itself.

So the effect's two halves have to be judged separately.

## Decision

**GPUI draws the Low tier: the epaint rings, and no halo. The seam does not
port, and `Quality` collapses to one tier.**

The halo's *shape* is not the obstacle. `src/fx/halo.rs` rasterizes the same
falloff on the CPU and matches the WGSL's own readback to **max 1/255** over
44x44 samples (16 pixels differing, all at the rounding boundary) — the two
renderers are the same effect.

The *blend* is the obstacle, and it is not a small difference. Measured on a
real map frame at the console's own strengths:

| measure | value |
| --- | --- |
| halo footprint that source-over darkens | **65.3%** |
| worst darkening (the toolkit's own pixels) | **44.5/255** luminance |
| max additive-vs-source-over channel gap | **130/255** |
| CPU composite of the same comparison | 61.7% darkened, worst 46/255 |

That third column is the whole argument: additive adds light, source-over
*replaces* what is behind it, so it attenuates the map. ADR-0015's load-bearing
property — "additive into a non-HDR target so it cannot clip badly or darken
anything behind it" — is exactly what the substitute gives up. A halo that
dims the coastline it is drawn over is not a weaker version of this effect; it
is a different one, and worse.

`Quality::Low` is already defined as the code that shipped before the halo
existed, so this is not a new degradation invented for the port. The three
states keep their channel; the rings and the symbol glyphs carry them.

## Rejected

- **A source-over sprite as the GPUI halo.** Numbers above.
- **A custom element drawing its own pass.** No device access in 0.2.2.
- **Keeping a two-tier `Quality` in the GPUI shell.** With no shader, the tiers
  would be decoration; the honest statement is that the effect is absent.
- **Pre-multiplying the sprite to make it read correctly.** That is a workaround
  for the *wrong* blend, not a way to the right one.
- HDR bloom and glass: rejected in ADR-0015, unchanged.

## Consequences

- `src/fx` stays egui-only until GPUI can express a pass or a blend mode. It is
  **not** deleted: ADR-0015's seam is still the client's, and the egui console
  still draws its halos.
- `src/fx/halo.rs` is new, and pure: `fade`, `Halo` tints, `HALO_R` and a CPU
  `rasterize`, with no wgpu in the file. The WGSL and the CPU raster read the
  same definitions, so they cannot drift, and the effect's arithmetic outlives
  the toolkit that currently draws it. `src/fx/mod.rs` re-exports the moved
  names, so no caller changed.
- The halo's invariant is now pinned on both sides: five unit tests assert the
  centre is full tint (channel sum 483, the same number `p5-epaint
  --fx-selftest` asserts on the GPU), zero at and beyond the inscribed circle,
  a monotonic ramp between, and that alpha tracks the additive weight.
- **Revisiting this is cheap.** `proto/gpui-island` with `PROBE=1` draws the
  same map twice in one frame, once with a source-over sprite, and the capture
  can be checked against `tint*a + dst*(1-a)` with no cross-run alignment (the
  compositor's window stacking makes two-run A/B unreliable, which is why the
  probe is one frame). If GPUI grows a per-element blend mode, that harness
  answers whether the halo can come back.
- The probe's first run caught a real trap: a premultiplied sprite composites
  to `a^2`, painting a dark disc instead of a glow. Any sprite work in this
  client must un-premultiply first, because gpui images are straight alpha.
