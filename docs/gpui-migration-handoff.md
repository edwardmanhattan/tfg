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

## U4d verification log — wheel chaining (2026-10-12)

**Native behavior found, then designed around.** GPUI's built-in scroll
handler adds the delta to EVERY scrollable under the cursor and never
stops propagation: nested scrollers move concurrently (the exact
two-surfaces-jump failure the chain exists to prevent). So the port
suppresses nothing and adds the column half: bodies scroll themselves
(the built-in no-ops, clamped, at its limit), and a band-level
`on_scroll_wheel` moves the column only when no open on-band body under
the pointer has room — `view::chained_wheel`, kernel-tested
(`one_gesture_moves_one_thing`, 13/13 view tests). Sign convention
verified in the Wayland client source: positive Y is content moving
down, same as egui, so the column subtracts identically.

**Wiring** (compiles; `Compiling` line in evidence): band container
carries `on_scroll_wheel` — refresh pointer, resolve body fit from the
live `ScrollHandle`, route via `chained_wheel`, clamp
`zone_scroll` to `[0, 364]` every frame. Positions render through
`pos − scroll` (drag math proven unaffected: the grab absorbs scroll).
Seeded `zone_scroll = 120` so the static layout exercises the
view-transform path (top cut + bottom cut) with no gesture.

**Pixels pending.** A fullscreen video occupied the only output during
verification; the window mapped behind it (hyprctl-confirmed) and was
killed to hand the screen back. Scrolled-layout capture + live-wheel
proof wait on a quiet machine. Nothing structural is unverified: the
decision is kernel-tested, the wiring compiles, the fits log live.

## U4e verification log — zone rails: origins, grip, z-order (2026-10-12)

**Kernel.** `view::zone_grip_rect` (10px strip centred on the inner
edge) + `view::zone_drag_width` (add on left dock, subtract on right,
then the rails) + tests both docks, rails, and the narrow-viewport
rail (`the_grip_straddles_the_inner_edge`,
`a_grip_drag_writes_the_live_width`, 15/15 view tests). The island
origins were already kernel (`zone_band`, `zone_island_origin`).

**Demo.** Positions now come from the rails — `zone_band` right-docks
the column at the edge gap, `zone_island_origin` stacks it — instead of
literals (identical numbers: 376/56/358/660, now derived). Grip div on
the inner edge with the 2px line (cyan dragged / grey hover / 35%
idle), `ResizeLeftRight` cursor, drag writes `zone_w` through the
kernel rule. Band press promotes the island in `zorder` (the grip's
`move_to_top` as paint order); islands render back-to-front.

**Cursor API note.** `cursor_*` builders are macro-generated tailwind
names only (`cursor_pointer`, `cursor_move`…); anything else goes
through generic `.cursor(CursorStyle::…)`.

**Pixels pending (again).** Browser + live terminal work covered the
window on every attempt; address-form focus returned `ok` but did not
raise it. Killed to hand the screen back. Right-dock layout, grip ink
and z-order await a quiet capture; the math is kernel-tested and the
wiring compiled (`Compiling` line in evidence).

## U4e/bisect postscript — the double-offset bug (2026-10-12)

**Root cause found, and it was never overflow, wheel, loop, or paint.**
`island()` relativized y (`pos − band_top − scroll`) but not x (`pos.0`
raw) — harmless while the band sat at x0 (U4b), fatal the moment zone
math docked it at x376 (U4e): every island parked at double offset,
off-screen. The red-rect test proved the container pixel-perfect while
exonerating nothing; the tell was island text rendering ~300px right of
place. Fix: pass the band box, relativize both axes (10 lines).

**Bisect ledger (what each step actually proved):** wheel handler and
overflow exonerated on verified-fresh binaries; loop condition
exonerated by `live=3` in the log; bg-over-children exonerated by
red-removal; the unconditional-twin and root-twin proved `island()`
paints fine — which is what finally pointed at coordinates, not
painting.

**Capture (floated + focused window):** all three islands render in the
clip container — island 0 top-cut mid-row at the band edge (scroll 120
view transform), island 1 whole, island 2 band + one row then black
(band-bottom clip). Seeded pointer clobbered again (focus likely
delivers a cursor event), so rims rest — correct for cursor-elsewhere;
the lit state stands proven from U4b.

**Process rules hardened.** (1) Proto rebuilds: wipe
`target/debug/.fingerprint/gpui-island-*` + `touch` + `cargo build`,
require the `Compiling` line AND binary-newer-than-source, or the run
is void — cargo replays stale fingerprints across quick-successive
edits and cost most of this session. (2) Float (`hl.dsp.window.float`)
+ address-focus makes captures deterministic on a crowded compositor.
(3) Downscaled previews mislead (white panel, foliage, phantom
buttons); verify by full-res crops and pixel scans.

**Compositor API found while relocating.** The `hl` Lua surface is now
enumerated via error-message introspection: `hl.dsp.window` =
close,swap,bring_to_top,pseudo,clear_tags,drag,fullscreen,resize,move,
deny_from_group,tag,set_prop,alter_zorder,toggle_swallow,pin,signal,
float,center,fullscreen_state,kill,cycle_next; `hl.dsp.workspace` =
swap_monitors,toggle_special,rename,move,change_id (moves workspaces
across monitors, not windows across workspaces). No silent window
relocator found; class-form `focus` remains the capture tool.

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

## U6 spike — the map texture decision, measured (2026-10-12)

**Question.** How do 800x600 raw RGBA map frames reach a GPUI surface at
interaction cadence — URI churn or custom element? Surveyed pinned gpui
0.2.2 source first; two of the four candidate paths are dead on arrival:

- `ImageSource::Render(Arc<RenderImage>)`: `RenderImage::new` needs a
  `Frame` that is `pub(crate)` — external crates cannot construct one.
- `surface()`: macOS-only (CoreVideo). Dead on Linux.
- Zero-copy external texture: absent, as ADR-0002 already observed.

That leaves `ImageSource::Image(Arc<Image>)`: ENCODED bytes (PNG)
decoded async off-UI. New spike crate `proto/gpui-map` (own lockfile,
own target, same xattr patch mirroring) times the round trip with
synthetic frames (gradient + counter bar, 800x600 @10Hz, newest-wins
drain like the map thread).

**Verdict: stock path paints held frames, goes black under churn.**
`STATIC=1` (one frame, held): gradient + counter bar render full-bleed,
HUD `frame 1 enc 28.8ms` — captured floated + focused. At 10Hz
newest-wins: HUD advances (2,168 `ui frame` lines, queue ≈0.2ms) but the
surface stays black (full-frame saturation census: 0/22,400). Mechanism:
`use_asset` decodes async and returns `None` until ready, notifying the
view on completion — but each new `Arc<Image>` (id = hash of bytes)
restarts the load, so the notify always arrives after the swap. The
surface perpetually waits for a frame that is already stale.

**Second finding: atlas tiles leak on this path.** `paint_image` only
ever `get_or_insert_with`s (keyed by per-frame `ImageId`); removal runs
solely through explicit `drop_image` or the `image_cache` element —
neither fires for bare `img(Arc<Image>)`. Every burst frame would mint a
~1000x750-device-px GPU texture and keep it to window close. Unbounded
by construction.

**Numbers.** PNG encode 800x600 (gradient, near-worst-case entropy),
~8,600 samples: min 12.1 / p50 25.1 / p95 33.3 / max 102.6 ms — on the
producer thread, off-UI, so acceptable for BURSTY map traffic
(newest-wins drain on camera moves, `JUMP_PUMP` frames per gesture), not
for a continuous stream the pump never produces.

**Decision.** Stock `Image` path unusable as-is for map bursts. Next
spike (small, no new API): hold-until-ready — keep displaying the stable
`Arc` until the successor's `use_data` returns `Some` (~1-frame lag,
always painting), plus explicit `drop_image` of retired frames for a
bounded atlas. Custom upload element stays the follow-up ONLY if those
numbers demand it.

### Hold-until-ready result — the fix works, stock path adopted

Same crate, three changes: (1) drain writes `pending`, never `frame`;
(2) `render` gates on `pending.use_render_image(...)` returning `Some`,
promoting only then, so the STABLE frame stays painted through every
decode; (3) promotion retires the old frame on both leaking axes —
`cx.drop_image(old_ri, Some(window))` for the atlas tile and
`remove_asset` for the CPU asset-map entry (`fetch_asset` never evicts);
superseded *pending* frames get `remove_asset` too. Counters + a
`retire` log line make the claim falsifiable.

**Evidence, 356-frame run:** `promote 355 / retire 354` — exactly
off-by-one (the first promotion has nothing to retire), so every
displayed frame's tile is dropped on replacement; `enc 356 / promote
355` shows newest-wins discarding superseded pendings while always
promoting the latest. Capture: `frame 561 ... up 484 ret 483`, gradient
full-bleed, **no black patches** — the churn black is gone.

**Memory, which is the real proof.** RSS sampled every 10s across 855
promotions: 144120 / 147976 / 140608 / 144356 / 144832 / 144924 / 144924
/ 144956 / 144936 / 148612 / 141228 kB — flat, ±7MB allocator wobble, no
trend. Leaking would have cost ~1.9MB decoded + ~2MB atlas tile per
frame (≈3.3GB over that run). Flat RSS proves BOTH the CPU asset map and
the (in-process, Lavapipe) GPU atlas stay bounded. Steady state is ~1-2
tiles: the displayed frame's, plus the pending's once promoted.

**Encode cost is a setting, not a wall.** The 25ms p50 above is the
`image` default: `CompressionType::Fast` **plus `FilterType::Adaptive`**
(5 filter candidates tried per row). Pinning `FilterType::NoFilter`:
min 3.9 / p50 11.4 / p95 17.7 / max 28.5 ms — 2.2x cheaper, still
paints (promote 342 / retire 341). Bytes grow, but they only travel
memory-to-decoder, never a disk or wire. Budget ~12ms per burst frame on
the map thread, off-UI.

**Adopted for U6.** Hold-until-ready + explicit retire is the map texture
path; no custom upload element. Open items for the integration slice:
(a) the real producer holds `egui::ColorImage` (RGBA, premultiplied) and
must encode off-thread — same shape as this spike's encoder, but the
frame no longer needs `MapResp`'s `egui::ColorImage` at all; (b) confirm
the colour space matches what gpui expects (this spike's synthetic frames
are sRGB-agnostic, so gamma is unverified); (c) `Image::from_bytes`
hashes the whole byte buffer for its id — fine at burst cadence, worth a
glance if frames ever stream continuously.

### Real frames: the synthetic numbers did not survive (2026-10-12)

The gradient above is near-worst-case entropy, and two of its findings
were artefacts of that. `examples/map_burst` drives a real camera gesture
against the seeded cache and writes every frame, so the gate could be
measured on the thing it will actually carry.

**Real imagery is 5x cheaper to encode, and the filter search is not
worth skipping.** 8 frames of Jakarta at 800x600, same encoder settings:

| metric | synthetic gradient | real map |
| --- | --- | --- |
| encode p50, `image` defaults (Fast + Adaptive) | 25.1 ms | **3.1 ms** |
| encode p50, `FilterType::NoFilter` | 11.4 ms | 2.5 ms |
| bytes, defaults / NoFilter | — | 400 KB / 1.8 MB |

Real frames have large flat regions, so deflate is cheap and the filter
search earns 4.5x compression for 0.6 ms. The NoFilter recommendation
above is **withdrawn** for the map path: use the default encoder. (It
stays right for synthetic-looking content, which is why both columns are
recorded — the lesson is that the entropy of the picture decides, and the
map's is low.) Render + pump dominates the producer by two orders of
magnitude: **p50 368.6 ms** per gesture step on a warm cache, so the map
thread is pump-bound and the encoder was never the bottleneck.

**The gate on real frames, sustained.** 12.5 fps (loop of the 8 real
frames, `BURST_MS=80`) for 120 s: `promote 1472 / retire 1471` — the same
strict off-by-one — with RSS flat at 139-147 MB the whole way. Leaking
would have cost ~4.2 MB/frame here (400 KB PNG + 1.9 MB decoded + 1.9 MB
atlas tile) — roughly 6 GB. Two real 800x600 windows are on screen in the
capture (`frame 8 enc 0.0ms lag 39ms pend - up 6 ret 5`): JAKARTA BAY,
Kota, Penjaringan, the toll-road shields and the coastline all render.

**Latency, measured end to end.** Decode + notify lag on 400 KB real
PNGs: min 9.4 / **p50 38.6** / p95 46.9 / max 57.2 ms (`promote …
lag=` log). `Image::from_bytes` on the UI thread: p50 0.10 ms, max
0.5 ms. So the gate costs about one frame of lag versus egui's
synchronous `TextureHandle::set` — the analysis predicted exactly that,
and it is invisible at the map thread's own 368 ms pump cadence.

**Colour space, still open.** The real frames carry MapLibre's
`PREMULTIPLIED = true` RGBA; the capture looks right (no washed-out or
dark cast, labels legible), but "looks right" is not a gamut check. If a
tint shows up over the egui shell side by side, this is the first place
to look.
