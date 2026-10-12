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

**Recipe correction 2026-10-12.** Address-form focus worked twice, then
went permanently `window not found` for identical calls. Class-form works:
`dispatch hl.dsp.focus({ window = "class:foot" })` → `ok`. Prefer class
(and now `app_id: "gpui-shell"` is set, so `class:gpui-shell`). Always
verify with `activeworkspace` after focusing — one capture round raced a
stale buffer and showed terminal text over a `CONSOLE_NIGHT` histogram.

**Slice 3 (`gpui-shell` + shared poll thread), verified 2026-10-12.**
`TFG_SCENARIO=surge`, crop 960x1048+960+32: fill `(15-18, 23-26, 41-43)`
(`CONSOLE_NIGHT` ±1), label reads `tfg // gpui shell` / `60 fixes` —
backend thread → channel → 250 ms drain → `notify()` → re-render,
end to end. Observation, not a defect yet: foliage at the crop edges —
the surface may not fill the tiled area (possible transparent
`WindowBackgroundAppearance` default). U-next: set opaque background and
re-crop tighter.

**Correction 2026-10-12.** No defect: repaged histogram of the same crop
is 985,132/1,006,080 fill (0.979) `CONSOLE_NIGHT`. The fringe was a
misread of the downscaled preview. `Opaque` is already the GPUI default
(verified in pinned source), and no background change is needed.

## U4a verification log — drag handle + close-last (2026-10-12)

**Kernel.** `view::clamp_island_to_viewport` ports the drag half of
`chrome::island_owned:374-390` (delta added by the caller, clamp here,
drags only — column positions and scroll stay out). Pinned by
`a_drag_clamps_to_the_viewport` (10/10 view tests green), including the
oversize-island pins-to-origin case.

**Island proto.** Band is the drag handle (left press records
cursor-minus-pos; a full-window transparent overlay rendered topmost
while held takes moves + release, so fast drags stay attached).
`pos` readout in the body. Close is an 18px `canvas` cross
(`BODY_SILVER` resting, `MAP_INK` hot — `paint_close` numbers) at the
kernel's `island_close_center`, rendered as the LAST child: GPUI hit
order is tree order, later siblings on top, which is the same rule as
egui's register-last-wins (`chrome.rs:489`). Moving it above the body
reproduces the exact shipped bug (lights up, swallows nothing).

**Capture** (island at 200,130, modal shut): dark body, 4 hull rows,
rim, `pos 200,130`. X center `srgb(139,139,139)` = `BODY_SILVER`
`(140,140,140)` at kernel coordinates; fill cluster `CONSOLE_NIGHT` ±1.

**Two findings.** (1) The scroll viewport paints its own opaque LIGHT
default — no ancestor bg shows through. First capture was a white panel
with invisible rows; explicit `.bg(CONSOLE_NIGHT)` on the scroll div
fixed it. Every future scroll surface needs an explicit bg. (2) Drag
motion itself is not live-triggered here (no mouse synthesis on
Wayland): clamp math is tested, the event wiring is code-reviewed
against `chrome.rs`. A live drag proof waits on a driver.

**Not done here:** zone band clipping, `BodyFit`/scroll-chain
measurement, sibling input ownership — the rest of U4.

## U4b verification log — zone band: clip, cull, one lit rim (2026-10-12)

**Kernel.** `view::island_on_band` mirrors `chrome::island_on_band`
(culled, never drawn off-band, and the caller needs the predicate
because off-band islands must not own input either). Chrome's six
reachability cases mirrored 1:1 into `an_island_shows_while_it_touches_
the_band` (11/11 view tests green).

**Demo.** Three islands in a full-bleed band (56–596): island 0 fully
on, island 1 straddling the bottom edge, island 2 wholly below (never
rendered). Ownership resolves ONCE per frame over open + on-band
islands; hover moves refresh the pointer. Band container clips and
paints nothing (a zone grows no background).

**Capture.** Hard FILL→DARK cut mid-body at the band edge (column scan:
fill y0–117, wallpaper-black from y118 — no rim, no AA slope, a
guillotine exactly where egui's clip goes). X at `BODY_SILVER` at kernel
coordinates (carried from U4a). Rims: island 1 top region holds 1,003 px
of α50%-cyan-over-night (the lit blend), island 0 holds 0 lit / 1,455
resting grey. Exactly one lit rim — ADR-0016's rule, pixel-proven with
the pointer seeded over island 1 (same precedent as the modal's
starts-open capture; the hover→resolve→rim path is real code).

**Open discrepancy, not resolved here.** `chrome::owning_island` takes
the FIRST containing rect (`.position`, "lower index wins") while
`view::owning_island` takes the LAST (`.rposition`). Identical on
non-overlapping columns — which is every layout today — but they
disagree the day two islands overlap. Needs a decision before any
overlapping layout exists; the port must not silently pick one.

## U4c verification log — scroll measurement (2026-10-12)

**The unknown is gone, and it dissolved rather than surrendered.** No
custom element is needed: GPUI 0.2.2 ships `ScrollHandle` (`offset()`,
`max_offset()`, `bounds()`, per-child bounds), fed during layout via
`.track_scroll()` on any stateful (`id`'d) scroll div. That is the whole
of `BodyFit`'s three numbers, fresher than egui's (layout-time, not
last-frame temp storage). Sign convention differs — GPUI offsets run
negative-down, the kernel positive-down — and flips at the shell
boundary, next to `rgba()`. `content_h` is `max_offset + viewport`.

**Kernel.** `view::BodyFit` + `can_scroll_up/down` + `BODY_SCROLL_SLACK`
mirror `chrome` 1:1, pinned by `a_body_claims_the_wheel_only_past_the_
slack` (12/12 view tests) including the Operator 98.4-vs-94 trailing-
nothing case. `chrome`'s `BODY_SCROLL_EPSILON` (0.5) is read by nothing —
dead on arrival, deliberately not mirrored.

**Demo.** Each island owns a `ScrollHandle` (`track_scroll`, ids salted
`("roster-body", i)` — one shared id would weld all bodies to one
offset, same rule as `chrome`'s `id_salt`). Twelve literal hull rows
overflow the 270px viewport. Change-gated `eprintln` per island is this
proto's `verify-scroll.sh`.

**Log proof** (stderr, no pixels needed):
`fit island=0 content=409 viewport=270 offset=-0 down=true up=false`
(and island 1 identically; island 2 logs nothing — culled, second proof
of U4b). Settles 0/0 → 409/270 within frames of launch. Viewport 270 =
300 panel − 30 band, exact.

**Process fix.** Two `cargo check` replays (0.22–0.46s) served stale
fingerprints after quick-successive edits and nearly cost a debugging
session (a stale binary's UI misread as a layout bug). Proto crates now
verify by `touch` + `cargo build` with the `Compiling` line in evidence.

**Proto-crate `[patch]` rule.** Cargo ignores `[patch.crates-io]` of a
path dependency, so `gpui-shell` resolved upstream `tokio-centrifuge`
(no `with_connect_data`) until the vendor patch was mirrored into
`proto/gpui-shell/Cargo.toml`. U8 will meet this class of issue in
reverse when the patches move into the workspace.

## U3 verification log (2026-10-12)

**Correction to the analysis first:** the shipped app does NOT vendor
Inter — it draws egui's stock fonts (Ubuntu-Light proportional, epaint
monospace with built-in Hack). The "four Inter families" line in
`docs/gpui-migration-analysis.md:4.3` describes an aspiration, not the
code. The Inter/JetBrains bytes live in `proto/p5-epaint/assets` (glass
experiment). So U3 is additive, not a port: register real fonts where
egui never had them.

**What landed.** `view` gains the five `TEXT_*_PT` sizes + `TEXT_SCALE_
LARGER`, mirrored from `tokens.rs` with rationale-by-reference, pinned by
`text_sizes_mirror_the_tokens` (9/9 view tests green). `gpui-shell`
registers Inter Regular/Medium/SemiBold + JetBrains Mono via
`add_fonts(include_bytes!(...))` (fatal on failure, so no silent system
fallback), draws the title in Inter SemiBold at body size with a
`FontFallbacks(["Hack"])` chain, and the count line in monospace with a
`→` arrow — the exact glyph class egui needed the `Hack` append for.

**Capture** (`TFG_SCENARIO=surge`, crop 960x1048+960+32): fill 0.978
`CONSOLE_NIGHT`, 1,103 bright px, three lines `tfg // gpui shell` /
`85 fixes` / `surge → java sea`, arrow present (not `.notdef`).
Typeface proof without a second capture: `fc-scan` shows the files'
internal families are `Inter` / `JetBrains Mono`, `fc-list` shows no
system Inter to shadow them, and registration `expect()`s — the pixels
can only be our bytes.

**Environment note.** `/tmp/opencode/lvp` (software Vulkan) vanished to
tmp cleanup mid-session (`NoSupportedDeviceFound` on the Intel iGPU —
blade rejects it, reason unexamined). Recreated without root by fetching
`vulkan-swrast` 26.2.2 from the Omarchy mirror and unpacking to the same
path; the handoff env recipe works unchanged. The durable fix is still
`yay -S vulkan-swrast`.
