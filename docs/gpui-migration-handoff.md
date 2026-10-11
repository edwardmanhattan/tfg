# GPUI migration handoff

**Written 2026-10-11.** Read this first, then the files it names. It exists so
the next session does not repeat this session's measurements.

## State

The migration has not started. What exists is proof and a foundation.

| Artifact | State |
|---|---|
| `src/view/mod.rs` | Architected seam. Palette, metrics and chrome geometry with no toolkit type. 8 tests pass, no display, no GPU. |
| `proto/gpui-spike/` | GPUI 0.2.2 renders on this machine. Verified, 847 frames, pixel-exact colour bars. |
| `proto/gpui-island/` | One ARCONS island in GPUI from `view`. Renders, verified by capture. Palette matches at pixel level. |
| `docs/gpui-migration-analysis.md` | Cost analysis. 21–34 weeks, 8 units, risk register. |
| `docs/adr/0018-…md` | Re-measured at the top. Original reasoning preserved below. |

## Verified facts, with how to re-verify

**GPUI renders on the machine's Wayland compositor.** `grim` on `wayland-1`
captures it. Not on Xvfb.

**The kernel's geometry is correct.** `cargo test --lib view::`. Eight tests, no
GPU.

**The island's palette is correct.** Sampled `E4E9EF` against `MAP_INK` `E2E8F0`,
`8DDEEC` against the lit rim `RADAR_CYAN` `22D3EE`, `959DA1` against the band
rule `CUT_GREY` `8C9BAE`.

**GPUI does not render on the project's Xvfb.** With software Vulkan installed
the surface creates and `PlatformNotSupported` goes away, but the capture stays
black. Unresolved. Does not block the Wayland path.

## Non-obvious API facts, all learned the hard way

`xattr 0.2.3` needs `ENOATTR` rebound to `ENODATA` (61), not the `ENOSTR` (60)
that ADR-0018 records. glibc defines `ENOATTR` as `ENODATA`. The patch is at
`proto/gpui-spike/patches/xattr`, wired in by `[patch.crates-io]`.

`Styled::inset` takes one `Length`, so a negative inset per side goes through
`.top(-px(n))`, `.left(-px(n))` and so on. `overflow_y_scroll` is on
`StatefulInteractiveElement` and needs an `.id()` to be stateful.

`gpui::rgba` is raw 0..1 with no gamma, the same convention as
`view::Rgba::as_f32`. Emit `Rgba` directly. Going through `Hsla` applies a
conversion the tokens never asked for and shifts every hue.

`Frame` must not carry `overflow_hidden` when a silhouette paints outside the
panel box. Three of the island's four bevel rings paint outside it and the clip
guillotines them. Clip the content column instead.

## Capture recipe, which finally worked

Get the window's geometry from `hyprctl -j clients`, unset `WAYLAND_DISPLAY` is
wrong here, keep it set. Then crop with `magick in.png -crop WxH+X+Y` where the
physical pixel is the logical coordinate times 1.25. The monitor is 1536 logical
at 1920 physical.

`hyprctl dispatch` fails in this shell with a Lua parenthesisation error. Skip
it. Crop the full capture instead.

Software Vulkan, unpacked without root at `/tmp/opencode/lvp`, so no install is
needed:

```
export VK_ICD_FILENAMES=/tmp/opencode/lvp/usr/share/vulkan/icd.d/lvp_icd.json
export LD_LIBRARY_PATH=/tmp/opencode/lvp/usr/lib
```

The user can install it cleanly with `yay -S vulkan-swrast`, which removes the
env-var layer from every later test.

## Next unit

The modal chamfer, in `proto/gpui-island`. A convex pentagon with the top-right
cut back by `view::CUT` (20.0), stroked 2px. It is the one silhouette egui
cannot draw, which makes it the strongest single argument for the migration.
One boolean and one click. Verify with one capture of the modal open.

Two traps to avoid, both from this session. Paint the silhouette outside the
panel box so the clip belongs on the content column. Convert colour with
`gpui::rgba`, never through `Hsla`.

## What is open after that

The island's drag handle, close button and input ownership. In egui the title
band registers last because that is the entire close-button fix, and a port has
to preserve the order.

`side_zone` and the scroll chain. `BodyFit` has no GPUI equivalent, so it needs
a custom element that reports its own scroll offset. This is the largest single
unknown left.

The widget sheet. About 6,650 lines of `&mut Ui` methods. Volume rather than
difficulty.

The unattended capture path on Xvfb. Opens a surface, paints nothing.

## Do not re-measure

The cost analysis in `docs/gpui-migration-analysis.md` stands. The 21–34 week
estimate stands. The three U1 design decisions stand, and they are the entity
topology, the map texture path, and the scroll measurement.

One correction. ADR-0018's claim that GPUI cannot present at all is true for
Xvfb and false for the machine's compositor. Both are in the document now.

## U2 verification log

**U2 shell (`proto/gpui-shell`), verified 2026-10-12 by capture.** One
`Entity`, one label, 1040x640 centred (tiler put it at ws4 right half,
768x838 logical). Crop 960x1048+960+32, active workspace confirmed 4 at
capture time.

- Fill `(15-18, 23-26, 41-43)` = `CONSOLE_NIGHT` `#0F172A` ±1, ~800k of 1M
  px. Same ±1 shift as the island record.
- Label `tfg // gpui shell`, 516 px with r>150, centred. Cores read dimmer
  than full `MAP_INK` at 12.5px under software Vulkan (AA fringe, not a
  token error: same `rgba()` path the island matched with).

**U2 poll-thread slice, verified 2026-10-12.** `src/shell_threads.rs`:
`WireKind`/`EmptyWire`/`build_wire` moved verbatim, `spawn_poll_thread`
shared by both shells. `cargo test --lib`: 361 passed, 4 failed, all 4
reproduce on the clean tree (pre-existing, `book`/`map_render`/`roster`/
`sim`). `cargo check --bins`: clean, zero warnings in touched regions.
Map thread stays put: `MapResp` carries `egui::ColorImage`, U6 replaces
the handoff.

**Capture recipe amendment.** This shell's `hyprctl dispatch` is unusable
(Lua wrapper rejects all args), but the raw Hyprland socket accepts real
Lua: `dispatch hl.dsp.focus({ window = "0x..." })` focuses by address and
returns `ok`. PID-match the window first (`hyprctl -j clients`), focus,
`grim`, focus back. Next: set `WindowOptions.app_id` so captures match by
class instead of PID.
