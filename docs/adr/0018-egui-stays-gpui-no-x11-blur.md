# ADR-0018: Stay on egui; GPUI buys no blur on this platform

## Status

Accepted. Reaffirms ADR-0002 against new evidence. No framework migration.

## 2026-10-11 re-measurement

The decisive blocker against GPUI was that it rendered no frames under this
project's capture display. That measurement is now stale and has been
re-run. It no longer holds.

The machine now runs a Hyprland session on `wayland-1` with a real GPU, and
the ICDs present are Intel, Haswell (hasvk) and NVIDIA, at Vulkan 1.4.357.
`grim` captures that output successfully. A GPUI 0.2.2 window with the
`xattr` patch applied opens, renders continuously, and composites with alpha
and real font weights. `proto/gpui-spike` holds the probe; it renders 800+
frames and a pixel capture of its three colour bars matches the specified
values exactly.

So the cost side of this decision is unchanged and the blocker side has
moved. The 707-package tree, the mandatory Vulkan and the unpublished
`gpui_platform` are all still true. The "GPUI cannot present at all" reason
is not.

This changes what a future migration has to argue about. It does not by
itself reverse the decision, because the live display is not the same
environment as the unattended one: the project's own capture path needs a
display that `import -window root` can photograph without a compositor, and
a Hyprland session is not that. Any port has to settle the verification
question before it starts.

The original reasoning follows, because the cost analysis is still the
analysis.

## Context

The Tern console port looked wrong next to its HTML mock, and the read was
that the toolkit was the cause. The question was reopened: port to GPUI, which
would make the layered translucency and the type weights possible, or stay on
egui and accept a ceiling.

Three things were measured before answering.

**The port was never set in the mock's typefaces.** egui ships
`Ubuntu-Light` as its proportional default and `Hack` as its monospace,
registered as `[Ubuntu-Light, NotoEmoji, emoji-icon]` and `[Hack, UbuntuMono]`
(`epaint-0.36.2/src/text/fonts.rs:490-523`). The mock is Inter with
`JetBrains Mono` carrying every data line (`proto/glass-mock/index.html:8`).
Neither was ever installed, so every label in the port rendered in Ubuntu
Light and every coordinate, clock face and unit name rendered in Hack.

That is a root cause, not a limitation. Advance widths were measured from the
actual TTF `hmtx` tables for the console's own strings: Ubuntu-Light is 7-11%
*narrower* than Inter at every size the console uses, while Hack and JetBrains
Mono agree to within 0.3%. The mono swap is advance-neutral and reflows
nothing; the proportional swap widens labels by about 8% and moves layout.

**egui's real ceilings.** `FontId` is `{ size, family }` with
`// TODO(emilk): weight (bold), italics, …` (`epaint-0.36.2/src/text/fonts.rs:21-28`),
so a weight is a font family picked by hand at the call site. There is no
`backdrop-filter` equivalent, so the mock's `blur(18px)` ground behind a
translucent panel cannot be drawn at all.

**GPUI, as it actually is.** Published as `gpui` 0.2.2 (2025-10-22), Apache-2.0,
repo `zed-industries/zed`, gpui a path member `crates/gpui`. Standalone use is
real: the published crate's own `examples/hello_world.rs` compiles against the
published crate and runs under X11 here, with no assets source
(`Application::new()` passes `Arc::new(())`, `src/app.rs:132`). `gpui_platform`,
which `main` requires, is not on crates.io.

## Decision

egui stays. GPUI is not adopted.

Reasons:

- **The blur motivation fails on this platform.** GPUI's
  `WindowBackgroundAppearance` has a `Blurred` variant
  (`src/platform.rs:1310`), but on X11 it only stores the flag and calls
  `update_transparency()` (`x11/window.rs:1380`); the real blur path creates
  `org_kde_kwin_blur` on Wayland (`wayland/window.rs:1133-1139`). This project
  runs on an Xvfb display (`scripts/ui-shoot/xvfb-up.sh`, `DISPLAY=:99`). On
  X11 GPUI is alpha-only, which is exactly what egui already does.
- **The cost is not free.** `cargo add gpui` pulls 707 locked packages; the
  build here did not even start clean, failing in `xattr-0.2.3` on `ENOATTR`
  missing from libc 0.2.190 on linux-gnu, via
  `xattr <- zed-async-tar <- gpui_http_client <- gpui`. With a one-line
  `[patch.crates-io]` it builds 520 crates and 3.1 GB of target. egui is one
  crate plus an optional Extras.
- **Vulkan becomes a hard requirement**, via `blade-graphics`, compiled
  whenever the x11 or wayland feature is on (`src/platform.rs:14-18`).
- **GPUI cannot render in this project's capture environment at all.** Built
  and ran here: the published crate's `hello_world` needs a
  `[patch.crates-io] xattr` to compile at all, and under the project's Xvfb it
  creates two 1x1 helper windows that never map, renders no frames, and idles
  in `epoll_wait`. The log's first line is
  `MESA: info: vulkan: No DRI3 support detected - required for presentation`,
  the same failure egui's eframe-wgpu hits. There is no software Vulkan on this
  box: no lavapipe, and the only ICDs are Intel, Haswell and NVIDIA hardware.
- **The one genuine advantage is real but partial.** GPUI has
  `FontWeight` (`src/text_system.rs:633`) that cascades through `Styled`
  (`src/styled.rs:404`), where egui needs weight-as-family. That is worth
  something and it is not worth a 707-package dependency, a build that needs
  patching to compile, and no rendering on the machine that judges it.

The type faces are fixed inside egui. Four Inter weights and JetBrains Mono are
registered under separate family names because egui has no weight axis, and
`Proportional`/`Monospace` are remapped onto them.

## Consequences

- The prototype's console is set in the mock's typefaces. Four Inter weights
  and JetBrains Mono are registered under separate family names, because egui
  has no weight axis. The mono swap is advance-neutral and reflows nothing; the
  Inter swap widens labels by 7-11%, so every hard-coded column offset in the
  port is now measured against the right face.
- **egui cannot present under the project's capture environment.** The Xvfb
  display has no DRI3, and eframe's wgpu backend needs Vulkan presentation to
  reach frame 21 where the capture is requested, so the shot hangs and is
  killed by timeout. This is a pre-existing condition, not new. `--shoot`
  succeeds intermittently (3s when the context does come up), which is why
  earlier captures varied between 480, 960 and 1920 wide.
- The blur gap is a known, unresolved gap, named in the console port. It is not
  a migration objective until the project moves to a Wayland compositor with a
  KWin blur manager.
- Weight support is the one place egui is genuinely worse than GPUI. If a
  future surface needs many weights, register them as families; do not reprice
  this migration on it.
- eframe API notes from this pass: `Context::set_fonts` writes definitions into
  memory and `begin_pass` applies them, so a font family registered during
  frame 1 is not bound until frame 2, and painting into it on frame 1 panics
  with `FontFamily::Name(..) is not bound to any fonts`. And a `FontFamily::Name`
  must have a binding in `families` as well as an entry in `font_data`, or it
  panics the same way on first use.
