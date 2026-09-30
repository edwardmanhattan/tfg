# ADR-0015: one shader seam, one effect

## Status

Accepted. First effect is the additive selection halo. Bloom and glass are
explicitly *not* approved (see Rejected).

## Context

The client is epaint end to end: tessellation on the CPU, one triangle mesh
to `egui-wgpu`, one render pass, painter's-algorithm ordering. That is the
right tool for a command centre and it has no room for a shader.

`egui_wgpu::CallbackTrait` is the seam that makes a shader reachable without
replacing the renderer. It gives a callback three phases — `prepare` before
egui's pass, `finish_prepare` after every `prepare`, `paint` inside egui's
own render pass — and command buffers returned from `prepare` are submitted
in the same `queue.submit` as egui's. So an offscreen pass is possible
without owning a device.

The design audit rejected most of what looked attractive:

- **Bloom** needs an HDR target and something above 1.0 to bleed, and egui's
  attachment is a plain swapchain. A real bloom chain is `prepare` plus an
  offscreen `rgba16float` — a renderer, not a widget.
- **Glass** fails the function gate outright: refraction behind a roster
  someone is reading is decoration on information-dense chrome.
- **A pulsing stale marker** fails the frequency rule. The wire drops in
  bulk, staleness is unattended, and an operator judging whether data is
  real does not need it flapping.

## Decision

Ship the seam and exactly one effect: an additive halo on the three marker
states the epaint rings already carry — selected, following, old data.

The halo reinforces an existing channel rather than inventing one. It is
static at rest, never pulses, and is additive into a non-HDR target so it
cannot clip badly or darken anything behind it.

Two design details are load-bearing:

- **The callback carries its own geometry.** `prepare` has the device and
  the queue but not `PaintCallbackInfo`, and `paint` has neither the device
  nor the queue. So the centre, colour and strength travel on the callback
  and the uniform is written in `prepare` from `ScreenDescriptor`. The
  pipeline's colour format travels with them, taken from egui's own
  `RenderState::target_format` — a mismatch is a wgpu validation error at
  draw time, so it is taken rather than guessed.
- **`Quality::Low` is the default.** It draws nothing, and because the
  effect is purely additive to the caller's paint, the layout is identical
  across tiers: the low tier is the code that shipped before this existed,
  not a second path.

## Rejected

- Full-resolution blur, anywhere. A 41-tap separable gaussian at 1080p is
  ~170M texture fetches; the same kernel at quarter resolution is 16x
  cheaper. Any blur added later must run at reduced resolution, and that
  constraint is why `Quality` exists at all.
- HDR bloom and glass refraction, for the reasons above.
- Instancing or a uniform array. One marker is one quad of six vertices with
  no vertex buffer, which is smaller than the machinery batching would need.

## Consequences

- `src/fx` is the only place in the client that touches wgpu types directly
  outside `gpuprobe`, and it is the only place with a shader.
- `Quality` is derived from `wgpu::DeviceType`, a typed enum, rather than
  from the adapter's name. Anything unrecognised is `Low`, because guessing
  wrong in the expensive direction costs frames on the machine that can least
  afford them.
- **Verification.** `proto/p5-epaint --fx-selftest` compiles the WGSL with
  the real driver, draws one halo into an offscreen target, reads it back
  and asserts the centre is lit and the off-radius region is not. Measured:
  peak channel sum 483, which is exactly the cyan tint at full strength, and
  0 outside. `cargo build` checks Rust, not WGSL — a bad shader only fails at
  pipeline creation at runtime, so this check is the point.
- **Not verified:** the pipeline format matching egui's pass inside the
  running console. The format is taken from egui so it should match by
  construction, and the shader is verified on this machine's GPU, but this
  is the one path in the module not run end to end.
