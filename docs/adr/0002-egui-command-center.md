# ADR-0002: egui for the command center, not GPUI

## Status

Accepted. Supersedes the GPUI spikes (removed).

## Context

The presentation client needed a command-center UI over the map viewport.
Two shells were built to parity (same fixture, same registry, same map
frame): a GPUI window and an egui window. Compared live.

## Decision

egui (via eframe) owns all client UI. GPUI is fully removed.

Reasons, observed rather than theorized:

- Immediate-mode roster code is a fraction of the entities/listeners shape
  and reads as a pure function of registry state — the game-loop fit.
- Paint callbacks give the future own-renderer a trivial seam; GPUI has no
  zero-copy external-texture path on wgpu backends.
- egui's loader/plugin model (image, svg, http) keeps the binary lean.
- Look, themed, was accepted as-is for this stage.

## Consequences

- `examples/map_window.rs`, `scripts/run-map-window.sh`, and the `gpui`
  crate are deleted. `map_spike` (headless tile check) stays.
- ADR-0001 (overlays, map engine stays pure) is unchanged; egui painter
  primitives are the same seam the GPUI divs were.
- eframe API note: 0.36 uses `App::ui` + `Panel`, and image loading needs
  `egui_extras::install_image_loaders` — both already in the shell.
