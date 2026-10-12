# ADR-0020: Cut the console over to GPUI; retire egui

U1–U8 are done. Every mechanism the console needs — window, fonts, island
chrome, the map surface and its raw texture path, the halo tier, and the
whole widget sheet — is proven in `proto/` at measured geometry against the
shipped console. The owner retired ADR-0018 and called for the cutover
(U8): build the GPUI console that draws the console, move the tests off
`egui::Context::default()`, and delete egui, eframe and egui_extras.

## Status

Accepted. Supersedes ADR-0018. ADR-0002 (egui command center) and ADR-0019
(the halo is the Low tier in GPUI) carry forward unchanged — this is the
cutover ADR-0018 deferred, not a reversal of the halo decision.

## Context — what changed since ADR-0018

ADR-0018 rested on four blockers. U1–U8 measured each:

- **"GPUI cannot present at all."** Stale. GPUI 0.2.2 renders continuously on
  this machine's Hyprland/Wayland session and `grim` captures it; the protos
  carry the evidence (U1–U7 verification log). The Xvfb gap is real and is a
  *verification-method* problem, addressed under Consequences.
- **The cost.** Unchanged and still true: `cargo add gpui` pulls ~707 locked
  packages, needs the `xattr` `[patch.crates-io]` line, builds ~520 crates and
  ~3.1 GB of target, and makes Vulkan a hard requirement via `blade-graphics`.
  `libvulkan1` is already a runtime dependency, so packaging costs nothing
  new. This is why the port is 6–10 weeks, not free — and it is a cost the
  owner has accepted.
- **The blur motivation.** Moot. GPUI has no per-element blend mode and no
  Linux zero-copy path, so the halo is ADR-0019's Low tier (rings, no
  additive halo) — accepted as the trade for the rest.
- **"A port has to settle the verification question first."** Settled: the
  capture path moves from unattended Xvfb `import` to the Wayland compositor
  (`grim` + a floated, focused window), which is what U1–U8 already used. The
  egui console's own intermittent `--shoot` is replaced, not lost.

## Decision

**Adopt GPUI as the console toolkit. Port the widget sheet from `impl
ShipApp` into a GPUI `Render`, reusing the `proto/gpui-island` panels and the
`src/view` kernel verbatim, then remove egui, eframe and egui_extras.**

- The `view` kernel (palette, metrics, chrome geometry, the map gate, the halo
  math) is toolkit-free by design and crosses unchanged — that was the point
  of the seam.
- The map thread, the `mpsc` channels, the poll thread (`src/shell_threads.rs`)
  and the local store stay exactly as they are; only the handoff changes, as
  U6 established.
- The 15 custom painters and the ~290 widget methods port one panel at a time
  under the same discipline as U1–U8: a real surface, real values, a capture,
  a number.

## Consequences

- **The verification method changes and is the real risk.** GPUI renders no
  frames under the project's Xvfb (`No DRI3 support`), and egui's `--shoot`
  was the unattended path. The replacement is the Wayland compositor capture
  (`grim` + `shot.sh`), which needs a live session. CI runs `tfg --probe-gpu`
  with `continue-on-error: true` — a runner may have no GPU. An unattended
  headless capture is deferred, not solved.
- **Vulkan is mandatory** at runtime on every platform through `blade`. The
  Linux package already carries `libvulkan1`; macOS uses Metal and Windows
  DX11/DX12. A machine with no Vulkan cannot run the console.
- **The halo drops to ADR-0019's Low tier** (rings, no additive halo). This is
  the one visual regression the cutover accepts, and ADR-0019 has the numbers
  for what source-over costs (65% of the footprint darkened).
- **The 53 tests in `main.rs` that build on `egui::Context::default()`** must
  move to the toolkit-free `view`/kernel tests before egui is removed — that
  is U8's first mechanical step and the reason the kernel was made pure.
- **Build cost lands on every contributor and CI run**: ~520 crates, ~3.1 GB
  target, the `xattr` patch. egui was one crate. This is the standing cost of
  the decision.
