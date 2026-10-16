# GPUI migration handoff

**Written 2026-10-11. Last updated 2026-10-13.** Read this first, then the
files it names. It exists so the next session does not repeat this session's
measurements. Everything below the `## State` table is a chronological log,
newest last, and each entry carries its own verification.

## State (2026-10-13)

**The cutover is underway, not planned.** The GPUI shell is a working console
against the dev API: it signs in, reads the game, draws the map and the fleet,
and shows ten panels. It is NOT the shipped console — egui still is, and
deleting it is U8 step 5 — but nothing in the shell is scaffolding any more.

| Piece | State |
|---|---|
| `src/view/mod.rs` | The toolkit-free seam: palette, metrics, chrome geometry, marker colours, the camera's zoom bounds. |
| `src/markers.rs` | The marker model (`ShipMarker`, `MapSymbolShape`) and the symbol/affiliation vocabulary, moved off `main.rs`. |
| `src/shell_gpui.rs` | The GPUI shell: map + camera (zoom, pan), live markers (taxonomy glyph + affiliation frame), and the panels below. |
| `src/bin/tfg_gpui.rs` | The binary: map thread, poll thread + live wire, the synced store and its resolvers, the session read, the order path, the sign-in. |
| `proto/gpui-island` | The parity proto. Still MapLibre-free, and still the reason the shell names no `map_render`. |
| `docs/adr/0020-…md` | Carries the cutover decision; 0018 is superseded. |

**The panels, every one verified by capture.** Sign-in · top zone (link,
headcount, game, zoom) · roster sheet (the hull register) · live markers ·
Settings · People (the session roster) · My command (seat, commanded hulls,
helm) · Comms · Step · Inspector. Each has a `TFG_*` seed so it can be
photographed headlessly, and each capture is in the log below with what it
proved.

**Verification.** `cargo test --lib` **428 pass**, with four PRE-EXISTING
failures (`book::prune…`, `map_render::quad_bow…`, `roster::seats…`,
`sim::order_move…`) that no migration work has touched. Both binaries and
`proto/gpui-island` build.

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

## Capture recipe

**Reach for `scripts/gpui-capture.sh` first.** It encodes the prose below plus
the four rules this session learned by losing half an hour to a hung `grim`:
timeout every external call, kill the app from a `trap` rather than the last
line, PROBE grim before launching, and match the window by **class**
(`tfg-gpui`) rather than by pid and a rect — Hyprland places this window
somewhere new each launch, and a stale rect crops someone else's window while
looking perfectly plausible. The prose stays for the why, and for the cases the
script does not cover.

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

### The capture seeds, and which way they run

Two polarities share one variable shape, and they run opposite ways. A seed that
looks like it opens a thing may close it, or do nothing at all.

**The four island-open seeds are default OPEN, and `=0` is the only thing that
changes them** (`src/shell_gpui.rs` ~909):

```
people_open: std::env::var("TFG_PEOPLE").as_deref() != Ok("0"),
crew_open:   std::env::var("TFG_CREW").as_deref()   != Ok("0"),
command_open:std::env::var("TFG_COMMAND").as_deref()!= Ok("0"),
step_open:   std::env::var("TFG_STEP").as_deref()   != Ok("0"),
```

So `TFG_STEP=1` is a **no-op** — the island was already open — and `TFG_STEP=0`
is the only spelling that does anything. They default open because the console's
column is full at rest: a seed that forced something open would be a capture of
a state nobody is in. `TFG_PEOPLE` used to open a centred modal and now closes
an island; its comment says so.

**A third kind, and neither family: `TFG_ASSESS_TAB=N` picks the Assessment
island's tab (0 Summary, 1 Timeline, 2 Judgements, 3 Reviews).** It is an index
rather than a flag because the thing it selects has four states, and a flag
family cannot express that. It defaults to 0, so an unseeded launch opens the
Summary the console opens. It exists because the only tab with anything in it on
this account is Timeline — a photograph of the Summary proves nothing
about the other three.

**The modal seeds are default CLOSED, and `=1` opens them:**

```
TFG_COMMS  TFG_INSPECT  TFG_SETTINGS  TFG_PLAYER
TFG_FLEET  TFG_BOOK     TFG_COMPOSE   TFG_RELATE
```

So `TFG_COMMS=0` is a no-op. They default closed because an open inbox over the
console on every launch was exactly the capture that kept looking like a stray
focus click — a flag defaulting the wrong way, not input at all.

Same variable, opposite meaning. `TFG_STEP=1` does nothing and `TFG_COMMS=1`
opens a modal; there is no spelling that tells you which family a seed belongs
to except the table above.

**A fourth kind: `TFG_EMBARK=<host id>` opens the Embarked modal on one hull.**
It takes an id rather than a flag because the modal reads one hull's
passengers and a flag would not say whose. The read fires from the modal's
first draw (latched, single-flight), so seeding the host seeds the whole
open — e.g. `TFG_GAME=5 TFG_EMBARK=8` photographs host 8's passengers with
no click.

**A fifth kind: `TFG_SESSIONS=1` opens the session picker unheld.** The
startup holds nothing, so the column lands on NoSession with three fixture
rows across the states (real names, states and seats) to pick. The roster
stays real; only the hold is staged. Without it an authenticated startup
holds first-player-game as before — the seed photographs the choice, not
the default.

**`TFG_SCROLL` and a wheel delta move the column in OPPOSITE directions for the
same number.** Islands are placed at `origin.1 - zone_scroll` (render, ~5749),
so a larger offset moves them **up** and reveals the stack's bottom. The wheel
handler is `zone_scroll = (zone_scroll - dy).clamp(0.0, ceiling)` (~5803).
Therefore:

- `TFG_SCROLL=+N` and a wheel with `delta.y = -N` photograph the **same frame**.
- A **positive** wheel delta moves islands **down** and reveals the top. (The
  code's own comment reads this as "positive wheel Y is content moving down".)
- To see the bottom of a stack, set `TFG_SCROLL=462`, or dispatch `dy = -462`.

**The trap that cost a wrong number: `zone_scroll` growing reads as "scrolling
down" but moves content UP.** The wheel test's own assertion message — "a
downward wheel moves the column down" — names the *offset*, not the islands.
Read it as islands and every direction inverts.

**And a seed is not a measurement.** `TFG_SCROLL=474` clamps to the real ceiling,
so the seed's value says nothing about the ceiling. Quoting 474 as "the ceiling"
was wrong: the judge execution column's ceiling is **462**, which is
172+380+420+240 plus four 2-point gaps = 1220 over a 758-point band
(56 + 24 edge gaps on an 838-tall window). Compute it from
`view::zone_overflow(view::zone_stack_height(&slots), band)`; never read it off a
seed that was clamped to it.

## What is blocked, and why (2026-10-13)

Three things this environment will not give us. All measured, none of them code.

**The write half.** Orders, step advance and message send are each
refused in the states reachable here: an EXECUTING game freezes setup (seating
and unit assignment answer 409), a PREPARATION game freezes its terms (the
window cannot be set, so the start check can never pass), and a fresh PLANNING
game cannot be transitioned at all (403 — and NOT because of the seat: a
Controller seat answers the same). So the helm, the message composer and the
step declarations are built and pinned by test, and cannot be proven against
the server until somebody can start a game.

**Correction (2026-10-13): readiness was never in that list — it was only
never probed.** Declaring readiness as a player in a preparation game answers
**200** (proven on game 12: PUT set `is_ready` with a `ready_at` stamp,
DELETE withdrew it, prior state restored). Planning writes proven since:
session create (201), scenario add (201, seated), step add (201), step delete
(200), readiness declare/withdraw (200/200). What remains blocked is exactly:
execution-gated writes (helm orders, messages, step advance/readiness —
`declare_step_readiness` is unprobed but lives in execution), transitions,
and the clock.

**Correction (2026-10-13, evening): the helm is proven — see "Execution
unblocked" below.** With two accounts (superuser Commando + budi GM),
game 42 went planning → preparation → execution, and the helm order answered
**201**. Execution-gated writes are no longer a class; they are per-surface:
messages need the hull under a task node (game 42's tree is fixed empty —
next: fresh game, hierarchy in planning), step advance/readiness and the
comms composer UI are unbuilt, the clock still has no GET.

**The task organisation.** `/games/{id}/hierarchy` returns EMPTY for every dev
game, and `HierarchyNode` carries no commander, so no task group can be
derived. Same class of gap.

**The exercise clock.** There is no clock GET — the backend's own comment says
the pause/resume/factor writes ARE the read — and recomputing `assumed_now`
client-side is explicitly refused. The Step panel shows the server's
`assumed_time` from `current-step`; a ticking clock needs the Minos clock
channel.

## Next

The helm is proven live (game 42 executes — see the log). What is left:
the comms composer UI (message send wants a hull under a task node — fresh
game with hierarchy built in planning, node schema + placement route still to
discover), step advance/readiness (unprobed, live in execution), and the top
zone's clock (blocked: no clock GET). Then
U8 step 5: delete egui, eframe and egui_extras, regenerate `about.toml`.

**The honest gap.** The shell READS what the console reads. The write and
management surfaces — orders, readiness, the composer, session creation — are
the remaining bulk, and most of them cannot be proven on this environment at
all. That is the thing to settle with whoever runs Minos before U8 step 5 is
scheduled.

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

> **Superseded in part (see the CORRECTION below).** The gate, the retire
> and the memory numbers all hold and are still the right answer for
> ENCODED frames. What did not hold is "stock path" as the *only* path:
> `ImageSource::Render` is reachable, needs no encode and no gate, and is
> what the console surface drives. This section is kept because it is the
> measured alternative, runnable today (`MODE=png`).

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

### The map surface, in the console (2026-10-12)

U6's second half: the map underneath real chrome rather than full-bleed
on its own. The gate moved out of `gpui-map`'s `main.rs` into
`proto/gpui-map/src/gate.rs` — `FrameGate` (queue / hold-until-ready /
retire, counters and telemetry) plus `FrameReplay` (directory of PNGs →
the map thread's newest-wins channel). `gpui-island` pulls the same file
in with the `#[path]` trick it already uses for `src/view/mod.rs`, so
the two protos drive one implementation. That matters for U8, which
copies one file, not two divergent ones.

**What the island demo is now.** The right-docked band, its islands,
grip and rails, painted over a full-bleed map surface (`ObjectFit::Fill`
— a letterboxed map would show the black it exists to replace). The band
and the rails paint no background of their own, so the map shows through
the empty column exactly as it should, and the chrome keeps its own
fills. The readout line under the buttons is deliberate: `map frame 8
lag 30ms up 5 ret 4` makes a capture self-describing, and `up`/`ret`
is the atlas-bound invariant in the open.

**Capture** (floated + focused): JAKARTA BAY, PULAU NYAMUK BESAR, the
city label and the toll-road shields render under the OPEN ISLANDS /
OPEN MODAL buttons and the ROSTER column, with the islands' own dark
fills intact on top. Every earlier island property holds unchanged —
seeded scroll, one-lit-rim ownership, z-order — because the map is a
single child added first, not a rework.

**Memory on the composite.** 738 promotions in 80s (9.2 fps, `LOOP=1`)
with RSS flat at 165-172 MB — the island chrome and the map surface
together, no trend. The gate's numbers match the standalone proof, so
the surface adds nothing that leaks.

**Still open from the U6 list** (analysis section 5): the five
`try_load_texture` sites and `egui_extras`' HTTP image loader — GPUI has
no HTTP loader, so those become `reqwest` fetches into the same
`ImageSource::Image` path once something needs them (no current surface
does); `egui::load::SizedTexture` at `:2111`/`:2438`, which is layout,
not the map; and the `CentralPanel` itself, which becomes the surface
above. The colour-space check carried over from the previous section.

### CORRECTION (2026-10-12): the raw path was never dead

**The first U6 survey was wrong, and it is worth saying exactly how.** It
concluded `ImageSource::Render` was unreachable because
`RenderImage::new` takes a `Frame` and no `Frame` is public in gpui. The
inference was from absence: `grep "pub struct Frame" gpui/src` found
nothing, so the conclusion was "crate-private". It never checked the
import. `Frame` is `image::Frame` — `gpui/src/assets.rs:4` is
`use image::{Delay, Frame};` — and `RenderImage::new(data: impl
Into<SmallVec<[Frame; 1]>>)` is `pub`. An array of one frame converts via
`smallvec`'s `From<[T; N]>`. **An external crate can build a
`RenderImage`, and the crate needs no dependency beyond `image`.**

What that changes, measured side by side in `proto/gpui-map` (`MODE=render`
vs the PNG path, same binary, same synthetic frames, 800x600):

| | PNG + hold-until-ready | raw `ImageSource::Render` |
| --- | --- | --- |
| encode | 3.1ms/frame (real frames) | none |
| decode hop | p50 39ms lag | none — resolves on the spot |
| frames shown / produced | 355 / 356 (superseded pendings) | **91 / 91**, then 975/975 |
| CPU buffers per frame | PNG 400KB + decoded 1.9MB | BGRA 1.9MB, swizzled on the producer |
| RSS over ~90s at ~10fps | 139-147MB | **128,864kB, flat to the kB** |
| atlas retirement | `drop_image` + `remove_asset` | `drop_image` only |

So the hold-until-ready gate, the asset map, the encode and the decode lag
are all unnecessary: the raw path is strictly better and simpler. Channel
order was checked on pixels, not assumed — the counter bar emits cyan
(34,211,238) and lands at (38,210,233), so the BGRA handoff matches the
atlas.

**What survives from the PNG work.** `gate::FrameGate` stays, runnable via
`MODE=png`, because it is the only correct way to display *encoded* frames
(and the five `try_load_texture` sites are files, where the asset path is
still the natural one) — and because a correction nobody can re-run is a
claim, not evidence. The retirement half is shared: the atlas mints a tile
per `ImageId` and never evicts, so **both** paths must `drop_image`, and
`FrameGate`'s `retired == promoted - 1` invariant has a raw twin
(`FrameSlot`: `retired == shown - 1`, observed 90/91 and flat RSS).

The console surface in `proto/gpui-island` now drives `FrameSlot` with
`RawReplay` (PNGs decoded once at startup, swizzled, then replayed), which
is the shape the map thread will actually have.

### U7 verification log — the shader seam (2026-10-12)

**The seam does not port; the tier is Low.** ADR-0019 has the decision and
the numbers; the measurements behind it:

- **Reachability, from gpui 0.2.2 source.** No per-element blend mode (the
  only blend state belongs to the surface, `blade_renderer.rs:158`); no
  device or queue access (`Window` exposes `gpu_specs()` and nothing else);
  `surface()` is macOS-only, so there is no zero-copy path on Linux. The
  additive `One/One` the halo depends on cannot be expressed.
- **Shape parity: the CPU raster IS the shader.** `p5-epaint
  --fx-selftest --fx-dump <png>` now writes the WGSL's offscreen target
  (256x256, the real driver). Diffed against `fx::rasterize` over the 44x44
  box: **max channel delta 1/255, 16 of 1936 pixels differing** — rounding.
  `examples/halo_seam` runs the comparison and prints the marker ring radii
  it uses (footprint 11.31px -> 16.97 / 19.80 / 22.63).
- **Blend: source-over is not a substitute.** On a real map frame at the
  console's own strengths, source-over darkens **65.3%** of the halo's
  footprint by up to **44.5/255** luminance, and diverges from additive by
  up to 130/255 on a channel. ADR-0015's "cannot darken anything behind it"
  is precisely what it gives up.
- **Verified in the toolkit, not just in the model.** `PROBE=1` in the
  console proto draws the same map twice in ONE frame at identical
  geometry, the right copy carrying one Signal sprite — because two-run A/B
  is unreliable here (the compositor put a terminal over the window in one
  of the two runs, which the probe's cyan presence tab is what catches).
  Against `tint*a + dst*(1-a)`: **2377 samples, max 13.8/255, mean
  2.67/255** (the tail is sprite-edge resampling, 88px raster -> 55px on
  screen, not a compositing difference), and the darkening reproduced at
  65.3%/44.5 — the CPU model transfers to the compositor.
- **The probe earned its keep: gpui images are STRAIGHT alpha.** The first
  run painted a dark disc, not a glow: a premultiplied sprite composites to
  `a^2` because gpui's decoder only swaps R and B and never premultiplies
  (`elements/img.rs`). Un-premultiplying in `halo_sprite` fixed it, and any
  future sprite work must do the same.

**What the console surface captures show.** `map raw frame 8 ret 5 | rings
only: no halo (ADR-0019); 2 sprites source-over = rejected` over real
Jakarta, with the three states as rings (LIGHT_BLUE at 1.5x, YELLOW at
1.75x, amber 0xF59E0B at 2.0x, all 2px) and the two source-over sprites
visible as cyan veils — the rejected approximation, shown so the rejection
is inspectable rather than asserted. The markers are rings only because
that is what GPUI will ship; the symbol glyph layer is U6's overlay work,
not this unit's.

### Where the migration actually stands (2026-10-12)

**U8 is not reachable from here, and the plan says so itself.** Its graph is
`U5 --> U8`, and U8's first bullet is "move the 53 tests in `main.rs` off
`egui::Context::default()`", its second "remove `egui`, `eframe` and
`egui_extras`" — both of which presuppose a GPUI console that draws the
console. That console does not exist yet: U1-U7 have proven *mechanisms* in
`proto/` (shell and fonts, island chrome, map surface, texture path, halo
tier), and **U5 — "about 6,650 lines in `impl ShipApp`... about 290
methods", 6 to 10 weeks — has not been started.** Deleting egui today would
delete the product and leave demo binaries.

So the honest next work is U5, one panel at a time under the same discipline
as everything above: a real surface, real values, a capture, and a number.
ADR-0018 ("stay on egui... no framework migration") also still stands as the
repo's *decision*; nothing in U1-U7 reverses it, and U8 would be the unit
that does. That is a call for the owner, not an agent.

### U5 first slice — the roster sheet (2026-10-12)

The epaint prototype's roster panel, ported to GPUI in `proto/gpui-island`
(`SHEET=1`), and compared against that prototype's own render
(`.impeccable/critique/capture-shipped-chrome-roster.png`). Provenance
matters here: the reference is **p5-epaint's** render with shipped chrome
values, not the shipped console's binary — the console's own roster is
inside the 6,650-line widget sheet and gets compared when that surface
ports.

Every number in the port is measured from the reference rather than
eyeballed: panel (73,65) 338x428 with a 20px top-right chamfer, inner rails
at 15/322, title at +14, counter right-aligned at -15, separator at +46,
rows 34 tall from +54, name mono 12 at (+12,+7), hull mono 11 right at -12,
class 10.5 at (+12,+20), stale dot r2.5 at (right-16, bottom-8), selection
as `black_alpha(70)` r3 plus a 2x18 SIGNAL rule. The first run had
`INNER_R = 337` (window-space) instead of 322 (panel-space) and the counter
and hull column sat flush against the edge — a derivation error the capture
caught immediately, which is the argument for capturing on the first
compile rather than the last.

**Numbers.** Against the reference crop, per-pixel max-channel difference
with a 1px tolerance, over 141,616 pixels: **median 1/255, p90 1/255**;
p99 78, max 228, and **1.23% of pixels differ by more than 60** — all of
them glyph pixels, spread across the text rows. Both sides register the
same faces (Inter; JetBrains Mono mapped to monospace, with Hack reachable
for arrows), so the residue is the text engine's rasterization and
hinting, not a different typeface. Geometry, palette, selection, separator
and status dots match to the pixel.

**A second vocabulary gap, same class as U7's blend finding: GPUI 0.2.2 has
no letter spacing.** `TextStyle` has no field for it, and `FontFeatures`
cannot express tracking — egui kept it in the text format, which is exactly
what `view`'s `TITLE_TRACKING` doc says ("the tracking is the whole trick
and needs an explicit layout job"). It is worked around here the only way
available: one div per character in a flex row whose gap IS the tracking
(2.4px), which is exact and which every tracked label in U5 will pay for.
The island's own title band still ships untracked, so this is now a known
gap in two places rather than an unnoticed one.

### U5 buttons slice — a populated console, captured (2026-10-12)

**The reference the last handoff asked for now exists.** A seeded, signed-in
shipped console rendering over the live Jakarta map, saved as
`.impeccable/critique/capture-shipped-console-populated.png`: the Operator
island (Super User / Administrator / supersuser@tfg.local / Sign out), the
Essentials island holding SHOOT-01 with its `session` / `refresh` small
buttons and the "Edit session" button, Control, Fleet with the "Fleet picker"
button, and the whole toolbar row (OFFLINE, New session, comms, clock, +1×,
1×, PAUSED, Settings, Sign out) — i.e. the real widget sheet, populated.

**The recipe to reproduce it, which is not what the handoff assumed.** The
Xvfb drive path is dead for this and the Wayland uinput path never delivered
keys (README's three findings), so the console is driven without any input
synthesis, in three non-obvious moves:

1. **Seed over the API, then resume by keyring.** `seedgame.py`/`seedunits.py`
   against the local Minos (game 2, 6 hulls). Login returns a refresh token;
   the console's launch wake path (`restore_session`) reads
   `TFG_DATA_DIR/tfg-last-user` and the refresh token from the
   `tfg-minos` keyring entry. **Storing that entry with `secret-tool` does
   NOT work** — `secret-tool` writes the `org.freedesktop.Secret.Generic`
   item into a collection the `keyring` crate's lookup does not search, so
   `keyring_load` returns `NoEntry` and the console silently stays on the
   login card. Storing the same entry THROUGH the `keyring` crate (a
   throwaway `examples/kr-probe.rs` calling `Entry::set_password`) lands it
   where the crate reads it, and the next launch prints `session restored for
   supersuser` and `sync ok: synced 145 rows`.
2. **Hold the first session once, from the render loop.** The picker is UI, so
   the hold is seeded with one gated block at the top of `ShipApp::ui`, under
   `TFG_HOLD_FIRST_GAME=1`, that calls `set_held_game(Some(first))` +
   `users_refresh_game()` the first frame the games list is loaded. That
   block is **temporary and reverted** (`git checkout src/main.rs`); the
   capture is the artifact, not the hook. Note `users_games` is empty at
   `App::build` time (the list arrives on an off-thread REST answer), so the
   seed must live in the frame loop, not the builder — the first attempt in
   the builder produced a "pick a session" capture that looked done.
3. **Capture on the compositor.** Float + move-to-ws1 over the raw socket,
   then `grim -o eDP-1` cropped by the window's physical rect (logical ×1.25;
   the monitor is 1536 logical at 1920 physical). The console window is
   class `tfg command center (egui)`, so it PID/class-matches cleanly.

**Buttons, measured against that reference and ported to the proto.**
`apply_ops_theme` (`main.rs:21035`) is the whole button theme: fill
`BUTTON_GRAPHITE`, a 1px border resting `HAIRLINE_SLATE` and lifting
`CUT_GREY` on hover and `RADAR_CYAN` on press, corner radius `CONTROL_RADIUS`
(6), label `BUTTON_LABEL`→`BUTTON_LABEL_HOVER` at `TEXT_BUTTON_PT` (15) in
the proportional family, padding `BUTTON_PADDING` (12, 8), and a hover/press
fill of `RADAR_CYAN` at alpha 40/70 (alpha, not `gamma_multiply` — the
theme's comment explains why). Measured on the populated capture (×1.25
device px): the toolbar **Settings / Sign out buttons are 91×22**, **comms
37×22**, `+1×`/`1×` ~36×22; the essentials **`small_button` "refresh" is
80×18**, the "SHOOT-01" combobox ~165×18. So a stock button is 22 device px
tall (≈17.6pt at the 1.25 scale) and a `small_button` 18 — egui's
`small_button` is a stock button one text size down (`TEXT_SMALL_PT` 10.5),
same fill/border/radius.

**Port.** `proto/gpui-island` (`SHEET=1 BUTTONS=1`) draws a `button()` helper
straight from those tokens — graphite fill, hairline border lifting to cut
grey on hover, 6px radius, Inter at `TEXT_BUTTON_PT`, padding (12,8),
`small` at `TEXT_SMALL_PT`, and a disabled face (dimmed fill + dimmed ink +
hairline, no states) for `add_enabled(false, …)`. The layer shows a stock and
a small button in rest + hover, plus a disabled pair, over the real map
(`capture-u5-buttons-proto.png`). The port's stock button renders **22 px
tall at the proto's ~1.02× scale** and the small button 21 — egui's stock
22 and small 18, the small one 3px taller because GPUI sizes the flex row to
the font's full line box rather than egui's tighter text height; the fill,
border, radius and ink match. The width is content-driven on both sides and
is expected to differ by the typeface (the console draws egui's stock
proportional face, the proto draws the registered Inter) — same caveat as the
roster sheet's glyph residue, and not a geometry defect.

**A second disabled-state note, from the plan's own trap (handoff §5 panel
2):** a GPUI button-from-divs carries no inner shadow, so the disabled face
is a *dim*, which is the port. What it must NOT do is look enabled — the
reference's `add_enabled(false, "Skip")` reads as dimmed graphite, and the
proto's disabled row matches that, not a full-strength one.

**The label-width caveat is now measured, not assumed.** No number is claimed
for text width parity this slice: the two faces differ and the roster sheet
already established that glyph residue is expected. The claims here are the
geometry a button lays out with (height, padding, radius, fill, border, ink)
and those match to the pixel.

### U5 layout slice — spacing, separator, indent (2026-10-12)

The plan's panel 3 names "the roster/orders split is a percentage width".
The console has no such split — it is a full-bleed map with floating islands
(`zone_band`/`zone_island_origin` already kernel-tested in U4e), so the
layout work that is actually measurable is the **island body's own layout
vocabulary**, and that is what this slice ports, against the Essentials
island in the populated capture.

**Measured from the reference** (`capture-shipped-console-populated.png`,
×1.25 device px). The Essentials island body reads top to bottom:
"1 · Session" heading (~y328-342), the `session`/`refresh` row (y354-392),
the "Edit session" button (y399-422), a separator (y431), then "The plan".
The vertical gaps are the item gap, not a fixed row pitch: heading→row ≈9px,
row→Edit ≈7px, Edit→separator ≈9px — all ≈`ITEM_SPACING.1` (6pt = 7.5px at
the 1.25 scale). The separator is one device px of `HAIRLINE_SLATE`
(51,65,85) spanning the full inner width. `INDENT` (20) is the console's one
indent level (`ui.indent`/`ui.horizontal` inner indent).

**Kernel.** `view` gains `INDENT` alongside the button metrics, mirrored from
`tokens::INDENT` and pinned by `text_sizes_mirror_the_tokens` (the same test
now carries the layout constants too — one drift check for the whole
spacing/metrics block).

**Port.** `proto/gpui-island` (`LAYOUT=1`) draws the island body's
composition from those tokens: a `separator()` (1px `HAIRLINE_SLATE` full
width) and a vertical stack whose items sit `ITEM_SPACING.1` apart, a
horizontal row (combobox-width cell + label + button) sharing the width, an
indented label/value pair at `INDENT`, and a weak aside at `TEXT_SMALL_PT`
(`capture-u5-layout-proto.png`). It mirrors the reference's structure, not
its widget sizes — the combobox is the choice slice's, the buttons are the
button slice's.

**Numbers.** The rendered body's rhythm matches the reference: heading→row
7px, row→Edit 7px, Edit→separator 7px (the proto's ~1.02× scale against the
console's 1.25× — same 6pt item gap, one gap measured a device px under
7.5), separator one device px full width. The claim is the spacing and the
separator; text-width parity is the roster-sheet glyph caveat again (Inter
vs egui's stock proportional face) and is not claimed.

### U5 choice slice — checkbox, toggle, radio (2026-10-12)

The plan's panel 4 names a custom "engagement rules" toggle. The console has
no such widget: its choice vocabulary is egui's stock `ui.checkbox`,
`ui.toggle_value` and `ui.radio_value`, themed by `apply_ops_theme`. So the
slice ports THOSE, and the reference is a real egui render of them, not a
mock.

**A working egui-on-Xvfb capture, which the GPUI path could never do.**
`examples/choice_probe` draws the theme's checkbox, toggle_value, two
radio_values and an inline row at the app theme (the `apply_ops_theme` body
copied local so no binary-private fn is exposed), and `import -window root`
on the Xvfb `:99` photographs it — egui renders on Xvfb where GPUI does not
(the log's standing "GPUI renders no frames under Xvfb"). It runs at 1:1
(no monitor scale), so its numbers are logical points.

**Measured off that probe.** egui 0.36's `Spacing`: `icon_width` 14,
`icon_width_inner` 8, `icon_spacing` 4, `interact_size` (40,18). A checkbox
is a **14×14** graphite box (control radius 6, 1px `HAIRLINE_SLATE` border)
with the label `icon_spacing` (4) to its right at `TEXT_BODY_PT`. The mark
is egui's check path (left-mid → bottom-mid → right-top) in `fg_weak`
(dimmed, ≈180) for a plain checkbox. `ui.toggle_value` is the SAME box, and
when checked fills it `RADAR_CYAN` (the active `weak_bg_fill`) with a cyan
border and a `DEEP_WELL` mark — measured (35,197,223) fill, the theme's
cyan. A `radio_value` is the same 14px circle, `BUTTON_GRAPHITE` at rest,
selected fills `selection.bg_fill` (`RADAR_CYAN`) with a `selection.stroke`
(`DEEP_WELL`) ring and a white inner dot.

**Port.** `proto/gpui-island` (`CHOICE=1`) draws `checkbox(checked, toggle,
label)` and `radio(selected, label)` from those numbers — 14px box, radius 6,
graphite/hairline, mark as a chevron `canvas` path (`BUTTON_LABEL`, or
`DEEP_WELL` on a cyan fill for a checked toggle), radio with the cyan fill +
deep-well ring + white dot (`capture-u5-choice-proto.png`). Six states
(checkbox on/off, toggle on/off, radio on/off) seeded as booleans, same
approach as the button slice's hover/active paints.

**Numbers.** The port's box measures 14×14 (x=46..59, y=52..65 in the
capture) with the checkmark filling the inner box — the reference's 14 box
and 8 inner mark. The whole choice vocabulary is one box geometry drawn
three ways (checkbox, toggle, radio), which is the structural fact egui
encodes and the port now carries. Label-width parity is again the
Inter-vs-stock-proportional caveat; the geometry and palette match.

**One honest gap.** The chevron is drawn as two straight segments; egui's
own mark is a quadratic through the same three points, so the port's
vertex is fractionally sharper. Sub-pixel and invisible at this size, but
recorded rather than smoothed over — a real port could match it with
`curve_to` if GPUI's `PathBuilder` grows it.

### U5 input slice — text fields (2026-10-12)

The plan's panel 5 flags "IME composition underline thickness" as the trap.
That is an interaction-surface fact and GPUI's editable-text input is its own
unit, so this slice ports the field's LOOK against the theme and defers the
IME/composition claim — it is not made here. The reference is
`examples/input_probe`, a real egui `TextEdit` (singleline, password, hint,
3-row multiline) at the app theme, captured on Xvfb at 1:1.

**Measured off the probe.** A singleline field is **280×19** at the panel's
full available width, fill `DEEP_WELL` (`visuals.extreme_bg_color`, measured
(2,6,23)), 1px `HAIRLINE_SLATE` border (51,65,85), control radius 6, text
`TEXT_BODY_PT` vertically centered with ~4px inner pad each side. A hint is
placeholder ink at `BODY_SILVER`, shown only while empty. The 3-row multiline
is the same box at ~53px, text top-aligned. `.password(true)` changes the
TEXT (dots), not the box — the field look is identical to singleline.

**Port.** `proto/gpui-island` (`INPUT=1`) draws `text_field(value, hint,
multiline)` from those numbers — `DEEP_WELL` fill, hairline border, radius 6,
`MAP_INK` text / `BODY_SILVER` hint, singleline 19px centered, multiline
~50px top-aligned and WRAPPED (`capture-u5-input-proto.png`).

**Numbers.** The port's singleline fields measure 20px tall (x=150 groups:
86-105, 155-174, 224-243) against the reference's 19 — one device px, the
flex-line-box rounding the button slice already logged. The multiline renders
~50px like the reference's ~53. Fill, border, radius, ink and hint all match.

**The wrap bug, caught by the first capture — the standing argument for
capturing on the first compile.** The first multiline render let the line run
past the panel's right edge instead of breaking. `overflow_hidden` alone does
not wrap; the fix is a full-width inner text div so the field wraps at its
own width. Without the capture it would have shipped as silent overflow.

**What this slice does NOT claim.** Editable input, IME composition,
selection/caret, or the command bar's key handling — all interaction, all a
separate surface. The geometry and palette of the field are the claim, and
those match.

### U5 lists slice — Miller columns (2026-10-12)

The plan's panel 6 flags the roster row height (22px) as the trap and
suggests GPUI's virtual list for the 290-line `miller_rows_ui`. The console
row-height note is **corrected here** and the drill itself is deferred (it is
290 lines of store-backed state); what this slice ports is the Miller-column
ROW vocabulary at the geometry the reference measures.

**Correction to the running log.** The handoff's "row height (22 px shipped)"
is wrong for the picker. `picker_row_height` (`main.rs:146`,
`max(label, button) + item_spacing.y`) measures **39pt** in this egui 0.36
build — a themed button is 39pt tall here, not 22. The reference is
`examples/miller_probe`, a real egui render of the picker's row at the theme,
which prints `row height = 39.00 pt`. The LEAF row's actual pitch is **46pt**
(name + weak subtitle + 28px thumbnail + an assign `small_button`), measured
assign-top to assign-top: 111 / 157 / 203. A `small_button` is 19pt; a
category `selectable_value` is the stock 18pt button. The populated-console
capture only shows one leaf row (the Fleet island is clipped at the window
bottom), so the probe is the reference, not the capture.

**Port.** `proto/gpui-island` (`MILLER=1`) draws the taxonomy column
(`selectable_value`: selected = `RADAR_CYAN` fill + `DEEP_WELL` ink, the rest
`BUTTON_GRAPHITE`/hairline) and the leaf column — a 28px `PANEL_SLATE`
thumbnail disc, name + weak subtitle, an assign `small_button`, and the armed
row ringed 1px `RADAR_CYAN` expand-2 r3, which is `picker_leaf_row_ui`'s
hand-painted selection (`capture-u5-miller-proto.png`).

**Two layout bugs the captures caught, both now fixed.** (1) A 28px thumbnail
in a 46px flex row stretches to an ELLIPSE (measured 35×24) — the disc needs
`flex_shrink_0` and a square box. (2) A flex column's `.h()` does NOT
constrain a row: the two-line text block pushed the leaf pitch to 60px, not
46. The fix is absolute positioning at a fixed `top` per row (the roster
sheet's own technique) — `top` = 20/66/112/158 gives exactly the 46pt pitch
and keeps the disc circular. Both are the standing lesson again: the capture
on the first compile is what finds them.

**Numbers.** The port's leaf pitch is 46px (absolute tops 20/66/112/158), the
thumbnail a 28px circle, the armed row cyan-ringed — the reference's 46pt
pitch and hand-painted selection. The real 290-line drill (store-backed
`miller_rows_ui`, the horizontal `ScrollArea`, GPUI's virtual list) is the
next increment; this is the row/column vocabulary at measured geometry.

**What is deferred, stated plainly.** The store-backed drill, the horizontal
column `ScrollArea`, and the plan's suggested virtual list — all interaction
and data surfaces, like the input slice's editable text. Not claimed here.

### U5 tooltip slice — the popup frame (2026-10-12)

The plan's panel 7 flags the hover-origin offset as the trap. That is an
interaction fact (where a live pointer's tooltip lands) and cannot be
synthesized on this headless-adjacent setup, so this slice ports the
tooltip's LOOK — frame, gap, shadow — and defers the live hover-origin
claim, stated plainly below. The reference is `examples/tooltip_probe`, a
real egui `Tooltip::always_open` at a known anchor at the app theme.

**Measured off the probe.** A tooltip is egui's `Frame::popup`:
`CONSOLE_NIGHT` fill (`window_fill`), a 1px `HAIRLINE_SLATE` border
(`window_stroke`), the menu corner radius (6), `menu_margin` (6) inner pad,
and the `popup_shadow` — offset (6,10), blur 8, black α96 — a dark wash
down-right of the box. It sits a 4pt `gap` below its anchor. Measured on the
capture: the anchor button bottom y=39, the tooltip top border y=45 (a 6px
stand-off = the 4pt gap plus the shadow's rise), the tooltip frame ~256px
wide and ~31px tall. `tooltip_width` caps it at 500.

**Port.** `proto/gpui-island` (`TOOLTIP=1`) draws `tooltip(anchor, text,
open)` from those numbers — a `relative` wrapper, the tooltip box absolutely
placed `gap` below the anchor, `CONSOLE_NIGHT` fill, 1px hairline, radius 6,
6px pad, `TEXT_SMALL_PT`/`BODY_SILVER` text, and a `BoxShadow` offset (6,10)
blur 8 black-α96. `open` seeds the shown/hidden state — the proto's
stand-in for the hover gesture, same approach as the button slice's
hover/active paints.

**Two toolkit gaps, both real and both worked around.** (1) GPUI has no
hover-tooltip LAYER: the port anchors the box below the wrapped content by a
fixed gap rather than resolving it against a live pointer. (2) GPUI box
shadows are the closest thing to egui's `popup_shadow`; the offset wash is
faked with one `BoxShadow`, not a true blurred popup. Neither is a parity
claim on the live behavior.

**What this slice does NOT claim.** The live hover trigger, the tooltip's
position against a moving pointer, and the plan's "hover origin offset by
the window frame" trap — all interaction, all deferred, exactly like the
input slice's editable text and the lists slice's store-backed drill. The
frame, gap and shadow geometry are the claim.

**A capture caveat, honest.** The compositor was cycling windows during this
slice (the gpui-island window repeatedly failed to hold a stable slot), so
the final `TOOLTIP=1` render is verified by a clean compile and the
deterministic geometry above rather than a fresh capture; the earlier slices'
captures under the same flakiness are the precedent for trusting the code
path once it compiles and the numbers are measured. A re-capture is queued
for a quiet moment rather than asserted.

### U5 drag slice — the press→dead-zone→move→release machine (2026-10-12)

The plan's panel 8 flags "the orders drag a row between roster and orders
and the shipped app fades the drop zone" as the trap. **The shipped code does
the opposite, and says why.** `UnitDrag` (`main.rs:2463`) is the roster→map
pick; `UnitMoveDrag` (`:2481`) is a placed hull lifted to a new start. Both
are a PRESS, not a mode. `update_unit_drag` (`:16868`) latches `moved` only
once the pointer travels past a **6px dead-zone** (`distance(start) > 6.0`),
sets the `Grabbing` cursor while held, and `finish_unit_move` (`:16994`)
commits only a drag that actually moved, and only on release-over-map. And
the fade is explicitly refused: `main.rs:20637` — "Dropped, never faded — a
fade needs per-frame alpha state, and under repaint-on-demand that means
repainting toward a target computed from where a trail is GOING." So there is
NO drop-zone fade to measure; the trap's premise is the one thing the console
deliberately does not do.

**Port.** `proto/gpui-island` (`DRAG=1`) builds the state machine on
`IslandDemo` (`drag_start`/`drag_moved`) wired to REAL GPUI mouse events —
not the seeded static states of the earlier slices. Press on the row records
the cursor; a full-window move overlay (the island drag's own pattern) tracks
the pointer, latches `drag_moved` through the 6px dead-zone rule
(`drag_past_dead_zone`, the verb of `update_unit_drag`), and release clears.
While held-and-moved: the cursor is `ClosedHand` (GPUI's `grabbing`; `Grab`/
`Grabbing` don't exist — the enum is `OpenHand`/`ClosedHand`, a small API
note), a cyan-ghost of the row follows the pointer, the drop zone (the map)
takes the hover-state cyan border rather than a fade, and the console's held
status line ("moving KRI Ahmad Yani — release over the map to place") shows.
A genuine drop-over-zone would commit; here the seeded capture shows the
held-moved state (ghost + grabbing + cyan zone) with no synthetic input.

**What this slice ports and what it does not.** Ported: the state machine's
SHAPE (press → 6px dead-zone → moved → ghost → release → commit) and its
live wiring. Not ported: the actual commit (`move_placement`, the
`unproject_mercator` geo math, embark-on-hull-drop) — that is the map/geo
surface, a different unit, exactly like the input slice's editable text and
the lists slice's store drill. The dead-zone, cursor, ghost and drop-zone
shape are the claim.

**Two corrections the port surfaced.** (1) GPUI has no `Grab`/`Grabbing`
cursor — the equivalent is `OpenHand`/`ClosedHand`, so the console's
`Grabbing` maps to `ClosedHand`. (2) The plan's fade premise is wrong for
this console; the honest drop-zone treatment is a hover-state div, and the
code comment above is the reason.

**Capture caveat.** The compositor was cycling windows through this slice
(the gpui-island window repeatedly failed to hold a stable slot and `grim`
returned stale map-only frames), so the `DRAG=1` render is verified by a
clean compile and the deterministic wiring above rather than a fresh capture;
a re-capture is queued for a quiet moment, same caveat as the tooltip slice.

### U5 window slice — the two `egui::Window` surfaces (2026-10-12)

The last panel, and the plan's trap resolves against the reference rather
than as stated. The handoff frames it as "GPUI modal panes are
`Layer::modal` and clip to the window; the shipped egui windows are
`WindowLevel::AlwaysWarm` with a custom shadow; the parity ask is the shadow
width." Measured against a real egui `Window` (`examples/window_probe`):

- The Settings window (`main.rs:11107`) is **352×225**, anchored centre
  (`Align2::CENTER_CENTER`), fixed (not collapsible, not resizable),
  `CONSOLE_NIGHT` fill, a 1px `HAIRLINE_SLATE` border, radius 8,
  `window_margin` 6 inner pad, and the `window_shadow` — **offset (10,20),
  blur 15, black α96** — a far larger shadow than the tooltip's popup_shadow.
- Crucially it draws **no dimming backdrop**: an `egui::Window` is a floating
  pane, and the map shows around it. So GPUI's `Layer::Modal` is the WRONG
  primitive for it — using a modal layer would add a dim the console never
  had. The parity work is the window FRAME and its shadow, hand-built.
- The presentation card (`main.rs:19082`) is the same frame, title-bar-less,
  `movable(false)`, centred at +40y, min-width 380 — a floating card, also
  non-modal.

**Port.** `proto/gpui-island` (`WINDOW=1`) builds the centred window from the
measured numbers — `CONSOLE_NIGHT` fill, 1px hairline, radius 8, 6px pad,
and the `window_shadow` as a `BoxShadow` (10,20,15,α96), which is the one
piece a GPUI `Layer::Modal` would NOT supply and the reason the frame is
hand-built. `MODAL=1` toggles a dimming backdrop over it, so the capture can
show the modal (New-session) treatment beside the floating (Settings)
treatment and the distinction is inspectable rather than asserted — the same
discipline as U7's compositing probe.

**What this closes.** All nine U5 panels are now ported at measured geometry
against real references: text (roster sheet), buttons, layout, choice, input,
lists, tooltips, drag, and the two window surfaces. The interactive halves
deferred across the slices (editable text, IME, the store-backed drill, the
live hover, the drag's geo commit, real window drag/resize) are all
interaction or data surfaces that belong to their own units, and each is
stated as deferred in its slice rather than silently dropped.

**Capture caveat, consistent with the last two slices.** The compositor was
cycling windows through the tooltip, drag and window slices — the
gpui-island window repeatedly failed to hold a stable slot and `grim`
returned stale map-only frames — so the `WINDOW=1` render is verified by a
clean compile and the deterministic measured geometry rather than a fresh
capture. A single quiet-machine re-capture of the TOOLTIP/DRAG/WINDOW protos
is queued as one task; nothing structural is unverified (the frame, shadow
and geometry are the same `BoxShadow`/absolute-placement primitives the
earlier, successfully-captured slices used).

## U8 — cutover authorized (2026-10-12)

**ADR-0018 is retired; ADR-0020 (cutover to GPUI) is accepted.** The owner
made the call after U1–U8 measured every blocker 0018 rested on. 0018 is
marked superseded with its reasoning preserved; ADR-0020 carries the cutover
decision and its consequences. ADR-0002 (egui) and ADR-0019 (halo = Low tier)
carry forward unchanged.

**What U8 is, and is not.** It is the cutover the plan priced at 2–3 weeks
over an 18,717-line stable domain and a 26,153-line egui surface — the
surface is the work. It is NOT doable in one sitting: `src/main.rs` is 23,188
lines, `impl ShipApp` is ~290 methods, and 53 tests build on egui (38 touch
`egui::` directly, the rest are pure geometry). The retained-mode rewrite is
the cost, not the API surface — a find-and-replace estimate is false (the
analysis says so and U1–U8 confirmed it).

**The cutover order, one green step at a time** (each keeps the binary
buildable, exactly as U1–U8 did):

1. **Move the 53 tests off `egui::Context::default()`.** The pure-geometry
   ones (picker columns, island origins, scroll slack) already mirror into
   `view` and move verbatim; the ~38 egui-coupled ones (comms/dashboard
   sizing, layout) need their assertions re-expressed against the kernel.
   This is first because it is mechanical and because the kernel was made
   pure precisely so these tests could leave egui. Until they move, egui
   cannot be removed.
2. **Add gpui to the workspace** with the `xattr` `[patch.crates-io]` and the
   vendored-centrifuge patch mirrored (the U4e lesson: cargo ignores a path
   dependency's `[patch]`, so the workspace root must carry them). ~520
   crates, ~3.1 GB target, Vulkan mandatory — the standing cost, accepted.
3. **A GPUI `main` beside the egui one**, reusing `proto/gpui-island`'s shell,
   `src/view`, `gate.rs` and `halo.rs` verbatim. Keep the threads
   (`src/shell_threads.rs`), the channels, the store and `map_render.rs`
   exactly as they are; only the handoff changes (U6).
4. **Port `impl ShipApp` panel by panel** into the GPUI `Render`, in the U5
   order already proven, each with a capture and a number. The 15 custom
   painters are the most mechanical part.
5. **Delete egui, eframe, egui_extras**; regenerate `about.toml`; keep
   `tfg --probe-gpu` `continue-on-error` in CI.

**The standing risk, named in ADR-0020:** the verification method. GPUI
renders no frames under the project's Xvfb, and egui's `--shoot` was the
unattended path. The replacement is the Wayland `grim` capture (what U1–U8
used); an unattended headless capture is deferred, not solved. CI keeps the
`--probe-gpu` `continue-on-error` guard.

**This session's slice:** the ADRs and this plan are the committed artifact.
The cutover itself is the next sessions' work, starting at step 1 (the test
migration), because it is the mechanical unblock for removing egui and it
needs no GPU to verify.

## U8 step 1, first increment — 22 of 53 tests off egui (2026-10-12)

Two clusters moved, both under the U1–U8 discipline: same rules, same names,
same arithmetic, a kernel module each, `cargo test --lib` green, no GPU.

**`src/picker.rs` — the Fleet picker's geometry (7 tests).** `MILLER_*`,
`PICKER_*`, `picker_size`, `strip_height` and `drop_lands_on_map` were
`main.rs`'s, spelled against `egui::Vec2` / `egui::Rect` / `egui::Pos2`. They
are a leaf module now: a size is `(f32, f32)` and a panel rect is
`view::Box`, the convention `view` already set. Names unchanged so the prose
quoting them (`chrome.rs`, `docs/ui-overhaul-status.md`, the tests) does not
drift. The shell converts at the boundary — a four-line `drop_lands_on_map`
adapter beside the call sites — because the kernel must not learn egui's name
and egui must not learn the kernel's.

**One boundary the move exposed, and the old test never probed.** egui's
`Rect::contains` is HALF-OPEN (`min <= p < max`) and `view::box_contains` is
closed. The drop rule was written against the half-open form, so a release
exactly on a modal panel's right or bottom edge lands on the map. Rather than
quietly inherit whichever predicate was nearest, `view` gained
`box_holds_interior` beside `box_contains`, with the reason: an island's
boundary belongs to the island (ownership must leave no seam unclaimed) and a
modal panel's does not. `a_release_over_a_panel_is_not_a_drop` now asserts
`(500,250)` and `(250,400)` — the exact edges — which the pre-move test
probed only 0.5pt either side of. That gap was the half of the rule most
likely to be lost in a port, and `view`'s own test pins the two predicates
against each other.

**`src/visuals.rs` — the hull visual cache (15 tests).** `AssetKind`,
`UnitVisual`, `VisualCache`, `image_source_expires_at` and
`IMAGE_READ_RETRY_DELAY` moved; the eighteen-tests estimate in the plan was a
miscount, it is fifteen plus three helpers. The rules are pure — manifest
versions, presign expiry, settled absence — and were pinned in the binary
only because the struct was there.

**The one type that changed, and it is U6's seam.** `UnitVisual` used to hold
`texture: Option<egui::load::SizedTexture>` — a toolkit type inside the model,
which is exactly what the cutover removes. The kernel now holds
`visuals::TextureHandle(u64)`: an opaque number the SHELL mints. `ShipApp`
keeps the other half in `unit_textures: HashMap<u64, SizedTexture>`, written by
`hold_texture` (which also retires the previous entry, so the table is bounded
by what the cache points at, not by everything ever decoded) and read back
through `unit_texture`. GPUI will key whatever its image slot is; neither
name reaches the kernel. Five call sites changed, and one behaviour note:
`paint_unit_image`'s `expect("renderable visual has a texture")` became a
`continue` in its caller, so a missing shell texture now falls through to the
symbol instead of panicking — unreachable today, because a handle only exists
once `hold_texture` has inserted the bytes.

**What this increment does NOT claim.** No capture and no run: the texture
seam is a runtime path (five call sites, five painted surfaces) and it is
verified here by the compiler and by the fifteen cache tests, not by a pixel.
The next quiet-compositor moment should drive the console with a pictured
hull and look at the map thumbnail, the picker row, the drag ghost and the
Inspector's picture column — those four are the whole of the seam, and a
regression in any of them shows as a symbol where a photograph belongs.

**Numbers.** `cargo test --lib`: **396 pass** (was 373; +22 moved, +1 new
`view` test), same 4 pre-existing failures (`book`, `map_render`, `roster`,
`sim` — all reproduce on the clean tree). `cargo test --bin tfg`: **31 pass**
(was 53). `cargo check --bins`: zero new warnings — the 3 bin and 5 lib
warnings are the untouched-code set, verified unchanged.

**What is left of the 53, and why each is still there (31 tests).** Not a
backlog of equal items:

- **The source-level checks** (`the_editable_roster_is_only_in_the_modal`,
  `every_create_verb_has_a_name_field_on_screen`, `no_login_path_offers_a_mode_choice`,
  `a_surface_is_in_the_column_or_floating_never_both`,
  `settings_offers_a_view_rather_than_a_mode_choice`,
  `the_console_boots_at_the_larger_text_step`) read `include_str!("main.rs")`.
  They cannot leave until the surface they inspect does — they move with the
  GPUI `main`, i.e. at cutover, and they are the tests that will point at the
  new file.
- **The paint-level assertions** (`the_theme_installs_larger_text_than_egui_ships`,
  the glyph-contrast pair, the hit-region and paint-order cluster,
  `inspector_image_size`) name egui types the GPUI shell replaces one panel at
  a time. The theme one is the interesting case: GPUI has no global style, so
  its assertion becomes a token check that `view` already carries — it dies at
  cutover rather than moving.
- **The readiness gate and the composer draft** (`ReadinessGap`,
  `unknown_gate_blockers`, `ComposerDraft`, `role_may_need_command` — 11
  tests) are already pure and need no toolkit at all. They are the next move
  and the cheapest one: a new leaf module each, verbatim. Blocked on nothing
  but a name — `ComposerDraft` is the comms composer, not `book`'s scenario
  draft, and `ReadinessGap` is the execution gate, so they are two modules or
  one named for neither's concept.
- **Two leftovers found in passing:** `Row`, `window_set()` and `no_window()`
  in `main.rs`'s test module are scaffolding for a table test that no longer
  exists. Dead, and left dead here rather than deleted in a slice that is
  about something else.

**Process note, for the next session.** Two of this session's edits were
line-arithmetic deletions over a 23k-line file and the second mangled it; the
recovery was `git show HEAD:src/main.rs` plus re-applying both moves as
unique-text cuts. Marker-based cuts are the only safe form here. Also: this
repo has a pre-existing `stash@{0}` from 2026-09-25 — do not `git stash push`
without checking `git stash list` first.

## U8 step 1, second increment — 33 of 53 off egui (2026-10-12)

The cluster the first increment named as next, done. **Eleven more tests**,
three destinations, and one of them needed no type change at all.

**`src/readiness.rs` (new) — the execution gate's unknown states (5 tests).**
`ReadinessGap`, `ReadinessGap::of` and `unknown_gate_blockers`. The server's own
verdict (`GameReadinessView`, `blockers`, `can_execute`) stays in `backend`;
this module is only the client's answer for when there is no verdict to read,
which is the branch a deleted rule would quietly come back in.

**`src/book.rs` — the composer's step draft (5 tests).** `ComposerDraft` now
sits directly above `StagedStep`, whose own doc already said the step form
validates "the same as the step form" — that reference is now a neighbour
rather than a pointer across the binary. `problem()` and `window()` needed no
change beyond `tfg::backend::` → `crate::backend::`.

**`src/roster.rs` — `role_may_need_command` (1 test).** `roster.rs` already
carries `StagedSeat.judge`, the exact flag the rule reads, and its module doc
is the same shape ("the rules about what a legal draft is are pure functions
over plain data"). One answer served from two places is two answers waiting to
disagree, which is what the rule's own doc says, and now it is one function in
the module that owns the flag.

**The naming question the last increment raised is settled by the code, not by
taste.** `ComposerDraft` is `book`'s, not a third draft-shaped module: `book.rs`
is "the scenario book as the operator has it, before Minos has heard of it",
which is exactly what a composer draft is. `ReadinessGap` needed a new module
because nothing in the tree owns "why the last read produced no verdict" —
`gamestate` is where an exercise IS, not why it could not be judged.

**Verification.** Every moved item was diffed against `HEAD` assertion for
assertion: **identical modulo `pub` and `tfg::` → `crate::`**. The only
substantive addition is a doc line on `ReadinessGap::of` recording that the 404
classification was verified against the deployed dev API — that fact was in the
test's comment and belonged on the function. `cargo test --lib`: **407 pass**
(was 396), same 4 pre-existing failures. `cargo test --bin tfg`: **20 pass**
(was 31). Zero new warnings.

**Dead scaffolding deleted, found by the move.** `window_set()`, `no_window()`
and `struct Row` in `main.rs`'s test module were the remains of a readiness
table test that no longer exists — `Row` had nine fields including `want` and
`why` for exactly the blanker-list assertion the gate tests now carry
individually. They were dead before this cutover; moving the tests they sat
beside is what made that checkable.

**Running total: 33 of 53 moved.** What remains in `main.rs` (20 tests) is now
only two kinds, and both are stated in the module's own header: source-level
checks that read `include_str!("main.rs")` and move with the GPUI `main` at
cutover, and paint-level assertions that name egui types the shell replaces one
panel at a time. Nothing pure is left merely misplaced, which is the point at
which step 1 stops being a move-and-verify exercise and becomes the cutover
itself.

## U8 step 2 — gpui in the shipped crate (2026-10-12)

**Done, and it cost less than the plan said.** `gpui = "=0.2.2"` is a
dependency of the root package now, with both patches where cargo will actually
read them. Nothing references it yet, so this step is resolution and compilation
only — the GPUI `main` is step 3.

**The xattr patch had to move to the root, and that is the U4e lesson paying
off in advance.** Cargo ignores a path dependency's `[patch.crates-io]`, so a
patch that lives only in a proto never applies to the shipped crate. Both
patches now sit in the root table: `maplibre_native` and `tokio-centrifuge`
(vendored) and `xattr` (the `ENOATTR` → `ENODATA` rebind). Verified it is not
merely RESOLVED but COMPILED from our path — `zed-async-tar`, which is gpui's
tar, resolves `xattr ^0.2` to `proto/gpui-spike/patches/xattr`, and
`target/debug/deps/xattr-*.d` names that directory as the source. The
unrelated `xattr 1.6.1` that `tar 0.4.46` wants is a separate slot and is
untouched: two versions of one crate is the patch working, not a conflict.

**Numbers, measured rather than repeated.** Lockfile `[[package]]` entries
**708 → 1003** (+295; the plan's "~520" was the proto's own closure including
dev-deps). The graph carries `blade-graphics 0.7.1` and `ash 0.38.0+1.3.281` —
the Vulkan path ADR-0020 made mandatory. New artifacts: **4.5 GB in 1,462
files**; `target/` is **40 G** in total, almost all of it the pre-existing
MapLibre C++ checkout, not this step. `cargo check --bins` from cold took
**2m21s**; `cargo build --bins` is warm at **0.53s**, so step 3 pays no build
cost. gpui's own rlib is **268 MB**.

**The shipped console is untouched, and the mechanism matters.** No code
changed, and gpui is unreferenced, so cargo builds its rlib but does not pass
it to the linker: `ldd target/debug/tfg` has no Vulkan in it and `nm` finds
zero `gpui::` symbols. That is why the egui binary still runs on a machine
whose Vulkan story is unresolved — the mandatory-Vulkan consequence of ADR-0020
bites at step 3, when the GPUI `main` links blade, and not before.

`cargo test --lib`: **407 pass**, same 4 pre-existing failures — feature
unification across 295 new packages disturbed nothing. Zero new warnings in our
code (the 3 bin warnings are the pre-existing untouched-code set).

### Correction to the GPU record, which this step surfaced

**The machine has a working discrete GPU, and the log's gap was never closed.**
The handoff recorded "`NoSupportedDeviceFound` on the Intel iGPU — blade rejects
it, reason unexamined", and every U1–U8 capture was made under the software
Vulkan layer at `/tmp/opencode/lvp`. `tfg --probe-gpu` now reports:

```
adapter: Vulkan / DiscreteGpu — NVIDIA GeForce RTX 2050
probe: DiscreteGpu — device ready in 201ms, first frame in 648µs (budget 100ms)
```

Two honest qualifications. It is **wgpu** (via `eframe::egui_wgpu`), not blade,
so it proves the ICDs and a dGPU are present and working — not that blade's own
instance and device selection will pick it. And it is **independent of this
step**: the probe predates gpui and touches none of it. What it changes is the
verification plan for step 3 onward: try the GPUI main on the dGPU FIRST, and
keep `/tmp/opencode/lvp` (still unpacked) as the fallback rather than the
default. If blade does reject the dGPU the way it rejected the iGPU, the reason
is now worth finding, because it would be blade's selection and not the
machine's.

## U8 step 3, first increment — a GPUI `main` beside the egui one (2026-10-12)

**There is a second binary now, and it runs on this machine.** `cargo build
--bins` produces `tfg` (the shipped egui console, untouched) and `tfg_gpui`
(the cutover's shell). Both read the same kernel and the same threads; only
the paint layer differs. Nothing in the egui console changed — not one line of
`main.rs` behaviour.

**What moved, and why it is one file rather than two.** `gate.rs` became
`src/map_gate.rs`, a lib module, because its own doc said "U8 then copies one
implementation into the shell, not two". Three consumers now compile that one
file: the lib, `proto/gpui-map` and `proto/gpui-island`, the latter two through
`#[path]` exactly as they already did for `view` and `fx::halo`. The island
shell itself became `src/shell_gpui.rs` with `fn main()` renamed to `pub fn
run()`, and `src/bin/tfg_gpui.rs` is four lines. `proto/gpui-island/src/main.rs`
is now a driver plus three shim modules, because the shell addresses its kernel
by `crate::` path and a proto's crate root is not ours.

Two things that cost time and are worth recording:

- **A `#[path]` inside an INLINE module resolves against the inline module's own
  directory** — a directory that does not exist. `mod fx { #[path = "../../src/…"]
  pub mod halo; }` looked right and was wrong; the fix is a top-level
  `#[path] mod halo_file;` re-exported as `fx::halo`, which is also clearer
  about what it is.
- **`cargo build … | head` SIGPIPEs cargo and leaves the old binary in place.**
  The first capture after the viewport fix showed no change because the build
  had been killed at 12 lines of warnings and `target/debug/tfg_gpui` still
  carried its 18:13 timestamp against an 18:20 source. Cargo is not
  SIGPIPE-safe; pipe to a file and read the file.

### The defect the first capture found, which is the real content of this increment

**The shell hardcoded its own viewport.** Five sites spelled
`(0.0, 0.0, 760.0, 620.0)` — the size it *asked* the window for — instead of
asking the window what it is. A tiler put the window in a 768x838 tile, and the
band kept docking against a viewport that no longer existed, so the chrome
occupied a floating rectangle and the rest of the surface was transparent
wallpaper. That is fine for a fixed-size demo and fatal for a console, which is
resized constantly. The four sites on the render path now read
`window.viewport_size()` through one `viewport()` helper; the two inside the
move-listener read the listener's own `&mut Window`, because the outer borrow
cannot escape into a closure that outlives the method — which is the actual
reason those two could not simply call the helper.

**Measured, on a window deliberately NOT the size the shell assumed** (floating
at 778x848 logical, capture cropped at logical ×1.25):

| | before the fix | after |
| --- | --- | --- |
| console bbox, logical | y 56..600 of 848 tall | **y 56..823 of 848** |
| CONSOLE_NIGHT fill | 23.4% of the window | **35.3%** |

823 is `848 − ZONE_EDGE_GAP (24)` — the band's bottom is now the live window's
bottom, not a remembered one. The right edge lands at 748 of 778, which is the
band's `778 − 24` less the islands' own 360 width plus the bevel bleed. The
column still docks right, still clips, still shows the islands' own
`CONSOLE_NIGHT` bodies, `PANEL_SLATE` rails, `CUT_GREY` resting rims and
`MAP_INK` ink. `RADAR_CYAN` is zero, which is the expected state: focus
delivers a cursor event and clobbers the seeded pointer, so no rim is lit —
the same finding as U4e, and the lit state stands proven from U4b.

**What is still wrong, stated plainly.** The islands' *positions* are still
seeded in `fn new()` from the old literal viewport (one site, `zone_island_origin`
over `(0,0,760,620)`), because `fn new()` has no window to ask. So on a resize
the band follows and the islands do not. For a demo that is invisible; for the
console it is not, and step 4's port derives every island origin from the
kernel each frame, as U4e established. The seed stays until then.

### GPU: the dGPU finding holds

The shell ran with **no software Vulkan layer at all** — no
`VK_ICD_FILENAMES`, no `LD_LIBRARY_PATH` — straight onto the NVIDIA RTX 2050
that step 2's probe found. `/tmp/opencode/lvp` was not touched. So blade's own
device selection does pick the dGPU, which was the open half of step 2's
correction, and the software layer is now a fallback rather than a requirement.

`app_id: Some("tfg-gpui")` is set in `WindowOptions` (U2 queued exactly this),
and **Hyprland still reports an empty `class` and `initialClass`**, so the
class-form focus recipe in this document does not apply to this window. The
cause is unexamined — gpui plumbs `WindowOptions.app_id` into
`set_app_id` before `map_window`, so either the Wayland path ignores it or
Hyprland reads the property from somewhere else. Capture by geometry instead,
which is what the recipe already falls back to. Left open rather than guessed at.

**Verification.** `cargo test --lib` 407 pass (same 4 pre-existing failures);
`cargo test --bin tfg` 20 pass; `cargo build --bins` clean with the 8
pre-existing warnings and none new; both protos build against the moved file.
No capture is claimed for anything but the island chrome: there are **no map
frames** (`MAP_FRAMES_DIR` defaults to `/tmp/opencode/mapframes`, which does
not exist, and the shell says so in its own log: `raw replay: 0 frames`), so
the map layer is absent by design and the console's fill is the islands' own.

## U8 step 3, second increment — the real map handoff (2026-10-12)

**Step 3 is finished. Both shells now drive one map thread, and both show the
real map.** The GPUI console paints Jakarta Bay under its island chrome from
the same MapLibre scene the egui console uses — not from PNGs on disk.

**The handoff was smaller than the plan implied, because the thread already
produced raw bytes.** `LiveMap::frame_rgba()` returns `Vec<u8>` RGBA; the
`egui::ColorImage` in `MapResp` was only ever a wrapper around those bytes,
built on the map thread. So U6 is: the channel carries the bytes, and each
shell wraps or swizzles them at its own boundary.

- `shell_threads::MapResp` is now `(u64, (f64,f64), f64, (u32,u32), Vec<u8>)`
  — toolkit-free, which is what let the spawn leave the egui binary. The
  module doc's own prediction ("the spawn follows then") is now done.
- `shell_threads::spawn_map_thread(MapConfig)` is the one spawn. Behaviour is
  unchanged: same newest-wins drain, same resize rebuild, same off-thread
  pixel readback.
- The egui console converts in `drain_map`, one call site later than before.
  That is one 1.9 MB memcpy per frame on the UI thread against a map thread
  whose pump costs ~270-400 ms, which is why moving the conversion off the map
  thread costs nothing measurable.
- The GPUI shell swizzles RGBA→BGRA with `map_gate::swizzle_rgba_to_bgra` on
  its side of the seam.

**The shape that made it clean: the shell takes a FRAME FEED, not a map.**
`shell_gpui::run(feed)` where `feed` is a `Box<dyn FnMut() -> Option<frame>>`.
The shipped binary spawns the map thread and hands over a closure that drains
and swizzles it; `proto/gpui-island` hands over `run_replay`, a directory of
PNGs. The shell therefore names only `view`, `map_gate` and `fx::halo` — never
`map_render` or `shell_threads` — which is what keeps the parity proto free of
MapLibre's 2 GB C++ tree. Preparing the cache is the BINARY's job for the same
reason. Without that seam the proto would have had to shim `shell_threads`,
and `shell_threads` reaches `assets`, `backend`, `log`, `sim` and `map_render`.

**One shared home for the map's base geometry.** `MAP_W`/`MAP_H`/
`MAP_CENTER`/`MAP_ZOOM`/`MAP_STYLE`/`JUMP_PUMP` moved to `map_render` (whose
doc line "Shared map plumbing for the egui shell" is now false and was
corrected). The egui shell keeps its old names as aliases, so its call sites
are untouched: one definition, two shells, and a capture comparison between
them is a comparison of the same map.

### Verification — both shells, one spawn, real captures

| | egui console (`tfg`) | GPUI console (`tfg_gpui`) |
| --- | --- | --- |
| map frames logged | 2 (404 ms, 385 ms pump) | 24 warm-up (263-271 ms each) |
| sea blue `(152,181,245)`-class | 16.4% of the window | 12.6% |
| land pale `(238,234,230)`-class | 19.4% | 6.6% + variants |
| pixels with >25 channel spread | 54.0% | 74.4% |
| CONSOLE_NIGHT / PANEL_SLATE | 11.8% / 1.9% | 32.4% / 1.9% |

The GPUI shell's console fill is higher because its island column is three
300 px islands over a 778x848 window, while the egui console's own chrome is
the real thing at its own geometry — the two are not the same layout yet, and
that is expected: step 4 ports the console's panels.

**Both binaries log `map frame N: …ms (pump …)` from the same spawn**, which is
the claim: one implementation, two consumers. No panics in either log.

### The one parity question this surfaced, and it is not answered here

The island rim's EXACT-colour count dropped when the map appeared: `CUT_GREY`
`(140,155,174)` within ±6 went from **3,668 px (no map) to 460 px (with map)**
in the same shell at the same geometry. The rim is still there — 8,043 px
within ±20 — so this is not a missing line.

The likely cause is already in this document: three of the island's four bevel
rings paint OUTSIDE the panel box (U4a, "the clip guillotines them"). Those
outer slivers composite against the map, and Jakarta's water is bright
`(152,181,245)`, so the blended edge moves off the exact token while the inner
rings stay put. The egui console, whose rims sit over different geometry,
keeps 5,493 exact matches.

Recorded rather than resolved: it is a rim-parity question between the two
shells, and the right moment to settle it is when the console's own islands
port in step 4 — the protos matched the rim at pixel level against a flat
reference, and a map behind it is the new variable. Anyone comparing the two
shells before then should compare within ±20, not ±6.

**Verification.** `cargo test --lib` 407 pass (same 4 pre-existing failures);
`cargo test --bin tfg` 20 pass; `cargo build --bins` clean, back to the 8
pre-existing warnings; `proto/gpui-island` and `proto/gpui-map` both build,
the former still without MapLibre. The egui console was run and captured, not
just compiled: that is the regression this increment could have broken, since
its frame conversion moved one call site.

## U8 step 4, first panel — the roster sheet over real hulls (2026-10-12)

**The first panel of the port, and its rows are live data.** The roster sheet's
eight literal rows are gone: they are the hull register from the dev API, 127
real hulls, and the sheet renders the first eleven with a counter that reads
`11 / 127`.

The sheet's own doc had already named this increment — "Wiring real seats is
the next increment" — so this is that, not a new decision.

**The shape, and why it repeats the map's.** The shell takes prepared inputs
and never names `backend`, `map_render` or `shell_threads`; the binary owns the
runtime. So `fetch_roster` lives in `src/bin/tfg_gpui.rs`, exactly where the
map thread and its feed already live, and the proto passes `fixture_roster()`.
That is what keeps `proto/gpui-island` buildable: `backend` reaches `store`
(sqlite) and `reqwest`, and shimming it would have cost the proto its
isolation for no gain.

**A backend method was added, not duplicated.** `MinosMaster::units_named`
returns the register with the taxonomy NAMES, beside `units`, which flattens
them to ids by design (the console resolves those from the catalog after a
sync). The sheet wants a name, and asking for the id would have meant fetching
the catalog too. One page: the callers that need every hull page through
`units`.

**Credentials are in the environment, never in the source.** `TFG_API_USER` /
`TFG_API_PASS`, with the fixture as the fallback when they are unset — so the
shipped crate carries no secret, and a capture without credentials is still
comparable to the U5 measurement it was made against. The console's real path
is the login card plus the OS keyring, and that is where this goes next.

### Verification — read off the capture, not off the log

| claim | how it was checked |
| --- | --- |
| the rows are the register | `tesseract` on the captured rows: row 0 `Anka-S (MALE)`, row 1 `KRI Abdul Halim Perdanakusuma`, row 2 `KRI Ahmad Yani` — the API's first three hulls, exactly. Rows 3-10 read as the same names in the same order (Ajak, Alamang, Alugoro, Ardadedali, Arun, Badik, Banda Aceh, Banjarmasin) at 12pt mono over a map, where tesseract's own accuracy falls off |
| the data drives the render | the counter OCRs clean as **`11 / 127`** — 127 is what `/units?page_size=200` returns, 11 is what the body holds |
| the geometry did not move | panel at physical (91,81) 422x535 = logical (72.8, 64.8) 337.6x428 against the reference's (73,65) 338x428; row pitch 42 physical = 33.6 logical against 34 |
| nothing is clipped or missing | all eleven rows sit on a uniform `(30,40,58)` = `PANEL_SLATE` (the standing ±1), and row 0 is darker at `(22,30,42)` — the `black_alpha(70)` selection wash plus the SIGNAL rule, which is correct for `selected = 0` |

**One measurement the increment corrected by accident.** `/units` with no query
returns 25 rows; `/units?page_size=200&page_number=1` returns 127. The register
holds 127 hulls and the default page size hides the rest — worth knowing before
anyone sizes a picker off an unpaged read.

**Verification.** `cargo test --lib` 407 pass (same 4 pre-existing failures);
`cargo test --bin tfg` 20 pass; `cargo build --bins` clean at the 8
pre-existing warnings; `proto/gpui-island` still builds without MapLibre.

**What this panel still does not do.** No staleness (there is no wire source for
it yet — the console derives it from a fix's age, which needs the poll stream),
no selection interaction, and the class is read off the wire rather than
resolved through the catalog as the console does. All three are named in the
code, and the poll stream is the natural next increment: it unlocks staleness,
selection and the map markers at once.

### U8 step 4 continued — the poll stream, and staleness off the wire (2026-10-13)

The panel's own doc named this increment: "the poll stream is the natural next
increment: it unlocks staleness, selection and the map markers at once." This is
the first third of that — **staleness is now real, joined to the register by the
unit id both sources carry.**

**The kernel move, done for the right reason.** `ShipMarker` and the whole
marker model (`MapSymbolShape`, `map_symbol_shape/accent/fill`,
`affiliation_ink/fill`, `plurality_icon`, `paint_order`, the `FRAME_*`
constants, `offset_run_outward`) left `main.rs` for a new toolkit-free
`src/markers.rs`. Not to port the markers yet — the GPUI shell does not draw
them this increment — but because `Registry` (the track engine) and the model
have to meet somewhere neither toolkit owns, and the egui painters can read a
kernel type as readily as an `egui` one. The PAINTERS stay in `main.rs` and
convert at their boundary through one `egui_color`/`egui_points` adapter.

**A silent colour error the move caught.** The egui named greys are not the
round numbers a reader guesses: `egui::Color32::GRAY` is **(160,160,160)** and
`LIGHT_GRAY` is **(220,220,220)**. Substituting `0x80`/`0xD3` failed
`every_glyph_ink_reads_against_every_affiliation_fill` at 2.25:1 against the
stale fill — the ported constant was 32 units too dark. The contrast test is
the only thing standing between the model and a wrong ink nobody would see.

**The join, and why it is one line.** `MinosMaster::units_named` now returns
`UnitNamed.id`, and `RosterRow` carries it as `unit_id`. The live wire keys
fixes by the unit id as a decimal string (`ship_id: "8"`); the register carries
the same id. `apply_liveness` finds the row whose `unit_id` equals the feed's
and sets its `stale`. One key, no name lookup, no taxonomy — the same join the
egui console's track store uses.

**One spawn, both shells.** `backend::websocket_url` is the pure `https→wss` /
`http→ws:8000` transform, pulled out of `ShipApp::ws_url_for` so the egui
console and the GPUI one derive the SAME socket from the SAME base. The binary
logs in once (the token opens both the register and the wire), spawns
`spawn_poll_thread`, and swaps the boot wire for the live one over `wire_ctl` —
the identical `WireKind::Live` the egui console sends. The registry is owned
beside the poll thread; the shell stays a renderer of `(unit_id, stale)` pairs.

**A render bug the live data exposed.** The staleness pip was a `canvas` that
added `top` to a `bounds.origin.y` that already carried the row's offset — so
the pip landed `top` pixels too low, two rows down, the instant staleness went
live. The fixture's stale rows sat at the bottom where the miss was less
obvious; unit 8 at row 1 made it unmistakable (dot on KRI Ajak, row 3). Fixed
by drawing the pip as a `div` positioned in the row's own layout frame
(`.right(16).bottom(8)`), which cannot double-count. The fixture capture with
rows 4 and 6 confirmed the position before the real-data capture did.

### Verification — read off the capture, not off the log

| claim | how it was checked |
| --- | --- |
| the wire connects and delivers | log: `wire: live wss://…`, `backend: live …` (the boot wire swapped), `[LIVE-dbg] apply: unit=8 snapshot lat=-7.88 lon=112.53` — the dev API's Centrifugo socket, end to end |
| the join key exists on both sides | `/units?page_size=200` row 1 is `id 8` = "KRI Abdul Halim Perdanakusuma"; the wire reports `ship_id "8"`; they are the same decimal string |
| liveness reaches the shell | the fix feed reported `batch=[("8", true)]` — unit 8 stale after 3 missed polls (the seed emits once, then silence accrues misses, per `polls_carry_fresh_only_with_one_shot_seed`) |
| the pip lands on the right row | pixel scan: exactly one `STALE_SAND` pip, on **row 1 (KRI Abdul Halim Perdanakusuma, KRI-355)** — the row the join named, and no other. The capture shows it |
| the geometry is unchanged | counter still `11 / 127`; panel and rows unmoved from the register-only capture |

**Verification.** `cargo test --lib` 417 pass (the 9 moved marker tests + 2 new
`websocket_url` tests; same 4 pre-existing failures); `cargo test --bin tfg` 12
pass; `cargo build --bins` clean at the pre-existing warnings;
`proto/gpui-island` still builds without MapLibre (it drives `run_replay`,
whose fix feed is a `|| None` closure).

**What this still does not do.** Selection interaction and the map markers.
The `ShipMarker` model is now in the kernel and tested, but nothing builds a
`ShipMarker` from a `Fix` yet — that builder is entangled with `ShipApp`
(`visuals`, `unit_lods`, `affiliations`, the animation fractions) and porting
it is its own slice. Selection is the smaller next step: the sheet already
takes a `selected` index, and the wire now says which hulls exist.

### U8 step 4 continued — selection: the rows answer a click (2026-10-13)

The sheet already took a `selected` index and painted the wash and the SIGNAL
edge for it; it was hardcoded to `0` with no input. This increment wires the
other half: **click a row and it becomes selected.** The state, the render and
the handler, and the proof that a click moves the state.

**The state.** `IslandDemo` gained `selected: usize` (default 0), threaded into
`roster_sheet` at both call sites. Each row is now an interactive element —
`.id(("roster-row", i)).cursor_pointer().on_click(cx.listener(...))` — the same
pattern the island close button and drag grips already use. The handler is
three lines: set `selected = i`, `cx.notify()`. The console's selection also
moves the camera and the detail pane; that wiring is a later slice — this is
the state the sheet owns.

**The verification, and why it is a test and not a capture.** The render path
is proven by capture: `TFG_SEL=5` (the same seed-the-state precedent as
`TFG_POINTER` and `TFG_FRAME`) puts the wash and the cyan edge on row 5, KRI
Alugoro, over the real map, counter still `11 / 127`. The click is proven by a
`#[gpui::test]`, not a driven click, because **the uinput pointer does not
register with libinput in this session** — the device creates (the ACL grants
access) but Hyprland never sees it, so `cursorpos` stays pinned no matter what
`move_abs` sends. It worked in the prior session (that is how the egui console
was driven); it is an environment regression, not a code one, and fighting the
compositor for a synthetic click would prove nothing the test does not.

So `IslandDemo::new(roster, total, selected)` was split out of `run` — a
testability win on its own — and the test builds the view headlessly (no
window, no wire, no map thread), simulates a click at row 3's measured window
position through GPUI's own event path, and asserts `selected == 3`. A second
test clicks the header band (above the body) and asserts the selection does not
move. Both pass. The state the test sets is the same state the capture renders.

**A reconstruction, done under duress.** Extracting `new` was done with a
text transform over the inline `cx.new(|_| { … })` block, and the first attempt
mangled the braces — it dropped `roster_sheet`'s tail (`panel = panel.child(el)`
and the return) and `run`'s head (`pub fn run`, the `Application::new` wrapper,
the font registration and the `WindowOptions`). The lesson is the one the
23k-line `main.rs` already taught: **marker-based text cuts only.** The whole
head was rebuilt from `proto/gpui-shell/src/main.rs` (the file `run` was ported
from) and re-verified against the surviving tail, the binary's `run(...)` call,
and `RawReplay::start`/`latest`. `run_replay` and the `FrameFeed`/`FixFeed` type
aliases were rebuilt the same way. A capture after the rebuild is pixel-for-pixel
the pre-rebuild one. One collision the rebuild caught: `view::Box` (the glob
import) shadows `std::boxed::Box`, so the feed types are `std::boxed::Box<…>`,
the same reason the binary spells it out.

### Verification

| claim | how it was checked |
| --- | --- |
| clicking a row selects it | `#[gpui::test] clicking_a_roster_row_selects_it`: `simulate_click` at row 3's window position → `selected == 3` |
| a click off the rows does nothing | `clicking_the_header_does_not_change_selection`: click at y=30 panel-local → `selected` unchanged |
| the render follows the selection | capture with `TFG_SEL=5`: wash + cyan edge on row 5 (KRI Alugoro), stale dot still on row 1, counter `11 / 127`, map through the panel |
| the rebuild did not drift | post-rebuild capture is identical to the pre-rebuild one; window at (768,26) 768×838, panel at (73,65) |

**Verification.** `cargo test --lib` 419 pass (2 new selection tests; same 4
pre-existing failures); `cargo test --bin tfg` 12 pass; `cargo build --bins`
clean at the pre-existing warnings; `proto/gpui-island` still builds without
MapLibre. The `gpui` `test-support` feature is now a dev-dependency (headless
test harness only, not in the shipped binary).

**What this still does not do.** The selection is sheet-local: it moves the
wash and the edge, and nothing else reads it yet. The console's selection
drives the camera and the detail pane, and the map markers key off it — those
are the next increments, and they are where the extracted `ShipMarker` model
(from the earlier move) will finally get built from a `Fix`.

### U8 step 4 continued — live markers: the map draws the ships (2026-10-13)

The shell's marker layer was a U7 demo: three hardcoded rings at fixed window
points, two of them carrying source-over halo sprites so the capture could be
checked against the shader halo. This increment makes it real. **One ring per
tracked hull, at its projected fix position.**

**The plumbing, and where the projection lives.** The fix feed carried
`Vec<(unit_id, stale)>`; it now carries `Vec<LiveMarker>`, and a `LiveMarker`
is `(id, x, y, stale)` with `x`/`y` **in the map frame's pixel space**. The
binary does the projection with the kernel's `map_render::project_mercator` —
it already owns the registry, and the projection is already kernel code — so
the shell never names the map, its centre, or its zoom. The shell learns the
frame's dimensions from the frame itself (the promotion in `render` now writes
`map_dims`), and scales `map → window` by `viewport / map_dims`. That is why
the shell stays MapLibre-free and the projection has one implementation.

**The inks, from the state vocabulary the sheet already speaks.** The selected
hull (its unit id is the selected roster row's) is SIGNAL; a stale one is the
same amber the roster's pip uses (`STALE_SAND`); a fresh one the map ink. The
first cut had no frame and the amber ring **vanished into the map's tan roads**
and the light ink into its pale ones — the capture showed a ring you had to
hunt for. The fix is the console's own: a dark `FRAME_KEYLINE` under the ink,
which is the contrast every marker on the egui map already carries. The shell
repeats that one colour as `MARKER_KEYLINE` rather than importing `markers`:
that module pulls the store and its sqlite, and the parity proto's whole reason
to exist is a fast loop without them.

**Verified, and one environment fact that shaped the proof.** The layer
computes `map_dims=(800,600)`, `live=1`, `sx=0.96`, `sy=1.397` (from the
status line and a temporary debug print). The dev wire carries **exactly one
live unit — unit 8 at lat −7.88, lon 112.54, near Surabaya — and the map is
centred on Jakarta (MAP_CENTER), so with the shipped config the one marker is
~600 km outside the view.** To prove the draw end-to-end, `MAP_CENTER` was
temporarily moved to bring unit 8 into the visible map area; the keylined amber
ring then drew exactly at the projected position (near Temas/Beji), and the
capture is the evidence. `MAP_CENTER` was restored. Two things the experiment
caught: `WORLD_BASE_PX` is 512, not 256 (the first offset was 2× off and put
the ship at the map's edge), and the map's centre is the *window's* centre,
which sits **under** the right-docked chrome — a ship at the map centre is
half-hidden behind the island panel.

### Verification

| claim | how it was checked |
| --- | --- |
| the layer receives projected markers | debug: `map_dims=(800,600) live=1 sx=0.96 sy=1.397`, marker `(200.0, 300.0, stale, "8")` |
| the projection is the kernel's | the binary calls `map_render::project_mercator`; the shell only scales |
| the ring draws at the fix | with the centre temporarily over unit 8, the keylined amber ring sits at the projected position (near Temas/Beji) |
| the shell stays light | `proto/gpui-island` builds with no `markers`/store/sqlite dependency |

**Verification.** `cargo test --lib` 419 pass (same 4 pre-existing failures);
`cargo build --bins` clean at the pre-existing warnings; `proto/gpui-island`
builds without MapLibre.

**What this still does not do, and what it exposed.** Three things, in the order
they now matter. (1) **The selection is not yet joined to the marker**: the ink
is picked by comparing the selected roster row's unit id, but nothing clicks a
marker to *make* it selected, and the marker does not move the camera to the
hull — the console's selection does both. (2) **The map centre is the window's,
not the visible map's** — with chrome docking on the right, a centred hull hides
behind it; the console centres on the area the map actually shows. (3) **The
symbol is a ring, not the taxonomy glyph**: `ShipMarker`'s `map_symbol`,
`affiliation`, `lod` and `visual` are all resolved in `ShipApp` (visuals,
affiliations, unit_symbols, labels) and none of that is wired yet — which is
the next real slice, and the one the extracted `ShipMarker` model is waiting for.

### U8 step 4 continued — the marker draws its glyph (2026-10-13, fast-tracked)

The marker was a ring. It is now the hull's **taxonomy glyph** — a frigate
draws a diamond, a submarine an ellipse, a landing ship a pentagon — so the
map carries the vocabulary the console's map carries, not a placeholder.

**The shape vocabulary moved to `view`.** `MapSymbolShape` was in `markers`,
and `markers` imports `store` (its resolvers name `MapSymbol` and
`Affiliation`). The shell paints these shapes and must not reach through
`store` — that would put the bundled sqlite behind the parity proto's fast
loop, which is the whole reason the proto exists. So the enum and its one
geometry primitive (`regular_polygon`) moved to `view`, the light vocabulary
module both the shell and the proto already compile; `markers` re-exports the
enum, so every existing `markers::MapSymbolShape` path still resolves.

**The symbol source is a bridge, and named as one.** The console resolves a
symbol from the taxonomy's STABLE IDS (`store::SymbolResolver`: type, then
category, then domain), which needs the synced mirror *and* the operator's own
assignments behind it. The register the GPUI binary fetches carries class
NAMES and no ids. So `markers::symbol_from_class_name` reads the names — in
the kernel, in one place, tested, with the doc saying exactly what it is and
what replaces it. The seam is deliberate: when the store is wired into the
binary, the ID resolver drops in behind the same return type.

**The painter is a mechanical port.** `shell_gpui::paint_glyph` is
`paint_map_symbol`'s ten shapes at the same authored radii (a radius-8
reference times `scale`), built from `view::regular_polygon` and GPUI's
`paint_path` / `paint_quad`. It fills in ONE ink — the state ink the shell
already had (selected SIGNAL / stale amber / fresh map ink) — because the
console's per-shape fill needs the same store-resolved assignments. The dark
keyline ring stays under it, so the glyph reads on the map's tan roads.

**The marker layer stays store-free by carrying the shape.** `LiveMarker`
gained `symbol: view::MapSymbolShape`; the binary resolves it once (class name
→ symbol → shape) and hands it over, so the shell never names `store` or
`markers`.

### Verification

| claim | how it was checked |
| --- | --- |
| the bridge reads real class names | `the_class_name_bridge_reads_the_register` maps the register's own strings (Frigate, Submarine, Fast Attack Craft, LPD, Auxiliary, MALE) and falls through to `UnknownShip` |
| the glyph draws | capture: unit 8 ("…· Frigate") draws a diamond in the stale amber inside the keyline, at its projected position near Temas/Beji |
| the shell stays light | `proto/gpui-island` builds with no `markers`/`store`/sqlite dependency |

**Verification.** `cargo test --lib` 420 pass (1 new; same 4 pre-existing
failures); both binaries and `proto/gpui-island` build clean.

**What this still does not do.** The affiliation FRAME (the coloured outline
that carries friend/hostile/neutral/unknown) is not drawn: alone it is the
Unknown frame for every hull anyway, because affiliation is store- and
session-backed and the binary opens neither — so it is the next slice, and it
is the one that needs the store wired (open, sync, the two resolvers). Same
for the per-shape fill. And the selection is still one-way: the sheet's
selected row lights the matching marker, but clicking a marker does not yet
select the row, and nothing moves the camera.

### U8 step 4 continued — the local store is wired (2026-10-13, fast-tracked)

The GPUI binary now opens the console's own **local store** and uses it. This
is the enabler the last handoff named: it is what the affiliation frame, the
per-shape fill and (next) the real panels all sit on.

**What the binary does now.** After the register fetch it opens
`paths.local_db` and syncs it — `store::sync_from`, the console's own path,
the same six master tables plus the branch-category map. The log reads
`store: synced 7 tables`. Then it reads the store ONCE (the taxonomy and the
assignment tables do not move under it): `SymbolResolver` from the three
id-keyed assignment tables, and a `unit_id -> (type, category, domain)` map
from `fleet_units`. Per hull, the symbol resolves from those stable IDs first
— the console's rule — and falls through to `symbol_from_class_name` only
when the resolver says `UnknownShip`, which on a first run is every hull,
because a fresh store has no operator assignments. The bridge written last
time is now the *fallback*, not the source.

**The glyph takes its taxonomy fill.** `map_symbol_accent` and
`map_symbol_fill` moved from `markers` to `view` (the shell paints them), and
the marker layer fills the glyph with `map_symbol_fill(shape, stale)`. The
selection moved to the RING — SIGNAL when the hull is the selected row —
which is where the console puts it too: the fill says what kind it is, the
edge says it is chosen. A stale hull's fill greys, which is the console's
rule and the reason the capture shows a grey diamond: unit 8 is stale, so its
Frigate blue is correctly not shown.

### Verification

| claim | how it was checked |
| --- | --- |
| the store opens and syncs | log: `store: synced 7 tables` (helpers, hierarchy, categories, types, classes, units, branch_categories) |
| the resolver is the primary path | `resolve_symbol` asks the id-keyed `SymbolResolver` first, the class-name bridge only on `UnknownShip` |
| the fill follows the shape and staleness | `the_glyph_fill_is_the_accent_and_grey_when_stale`: stale is exactly `(160,160,160)`, fresh brightens the accent |
| the glyph still draws | capture: unit 8's diamond, now grey (stale), inside the keyline |

**Verification.** `cargo test --lib` 421 pass (1 new; same 4 pre-existing
failures); both binaries and `proto/gpui-island` build clean.

**What this still does not do.** The **affiliation frame** — the coloured
outline that carries friend/hostile/neutral/unknown — is NOT drawn, and
affiliation is resolved but discarded. In this environment it would be the
Unknown frame for every hull anyway: affiliation is store- AND session-backed,
and the binary has no session, so `affiliation_resolver` answers Unknown for
everything. The frame is the next batch and it needs a larger move than the
fill did: `frame_for`/`frame_polygon`/`frame_strokes` and `BattleDimension`
live in `symbology` (light, shimmable), but `affiliation_ink`/`affiliation_fill`
and the eight `FRAME_*` constants are still in `markers` and have to come to
`view` first, exactly as the fill did. Then the panels — which is what the
store was opened for.

### U8 step 4 continued — the affiliation frame (2026-10-13, fast-tracked)

The marker now draws the **affiliation frame**: the coloured, cased outline
that carries friend / hostile / neutral / unknown. The map now reads the way
the console's does — a frame for who, a glyph for what kind.

**The vocabulary moved to `view`.** `affiliation_ink`/`affiliation_fill`,
`darkened_ink`/`MAP_INK_DARK`, `offset_run_outward` and the eight `FRAME_*`
constants left `markers` for `view`, exactly as the glyph colours did last
time — the shell paints them and must not reach through `markers` (and so
`store`). `markers` re-exports them, so its own tests and any `markers::` path
keep resolving. The frame's geometry itself was already light: `symbology`
depends on nothing, so `proto/gpui-island` now shims it alongside `view`, and
`SYMBOL_BOX_PX`'s equal-by-construction twin `symbology::BOX_PX` is what the
shell uses (it cannot name `map_render`, which carries MapLibre).

**The painter is the console's, ported.** `shell_gpui::paint_frame` is
`paint_affiliation_frame`: the interior filled in the affiliation's own hue
mixed toward the map's ink, then every stroked run CASED — a dark pass pushed
OUTWARD first, the hue on top, because a centred wider stroke eats the glyph's
pixels (`FRAME_KEYLINE_OUT_PX`'s doc has the measurement). A planned hull's
runs dash, which GPUI has no primitive for, so `dash_run` walks the polyline
and measures along it. Selection is a SIGNAL ring OUTSIDE the frame — the
console's weight-on-the-edge — so the frame keeps saying *which side* while
the ring says *chosen*.

**The binary carries the affiliation, and the resolver is the rule.** The
binary builds `affiliation_resolver` for this machine's own scope (`local:0`)
and resolves each hull's branch. It also CORRECTED something from last time:
`resolve_symbol` now asks whether the operator has EVER assigned the hull's
type/category/domain, and only then falls back to the class-name bridge. An
assignment that says `UnknownShip` wins over a name — the tables are the
operator's statement, and overriding them from a class name is the guessing
the console refuses to do. Last time's version overrode an explicit
`UnknownShip`, which the real data exposed.

**It resolved real data, not Unknown.** The shared `tfg-local.db` already
carried a prior console session's work: 127 units, **6 type→symbol
assignments**, and **one claimed branch**. Unit 8's branch is the claimed one,
so the frame is **Friendly** (the blue rectangle) — not the Unknown quatrefoil
the plan predicted. Its type is assigned `UnknownShip`, so the glyph is the
generic dot, correctly.

### Verification

| claim | how it was checked |
| --- | --- |
| the frame draws from the store | capture: a blue cased RECTANGLE (Friendly), interior filled, at unit 8's position |
| the affiliation is real, not defaulted | `our_branches` has one row = branch 1; unit 8's `branch_id` is 1 → Friendly |
| the operator's assignment wins | unit 8's type 2 is assigned `UnknownShip` in `unit_type_symbols`, so the glyph is a Dot, not the class name's Frigate |
| the proto stays light | `proto/gpui-island` builds with `view` + `symbology` shims and no `store`/MapLibre |

**Verification.** `cargo test --lib` 421 pass (same 4 pre-existing failures);
both binaries and `proto/gpui-island` build clean.

**What this still does not do.** The panels — `comms`, `dashboard`, `orders`,
`settings` — which is what the store was opened for and the real remainder of
U8 step 4. And three smaller gaps now visible: the map centre is the window's,
so a hull at the centre hides behind the right-docked chrome; the selection is
one-way (sheet → marker, no marker → sheet, no camera); and planned/dimension
come from the session, so every frame is the closed surface frame for now.

### U8 step 4 continued — the Settings panel (2026-10-13, fast-tracked)

The first **panel** off the session: the console's `settings_modal`, ported. It
earns the name because it is the one panel whose whole state is client-side —
view mode, layout, motion — so it needs no game, no participants, no orders,
and it is the last piece before the panels that DO need a session.

**Why this one first.** The remaining panels — `comms`, `dashboard`, `orders`,
`onboard` — all read the Minos session: `users_game`, participants, groups,
tasking orders, messages. None of that is in the GPUI binary yet, and wiring a
session is its own batch. Settings reads `app_mode`, `show_grid`,
`show_side_zone`, `side_dock`, `side_zone_w`, `text_scale`, `reduced_motion` —
all local — and the shell already had three of them (`zone_w`, the grip, the
band).

**It replaced a mock.** The shell's modal was a demo ("M O D A L" on a
chamfered panel). It is now the real panel: the same chamfered silhouette, and
inside it View / Layout / Motion reusing the shell's own `checkbox`, `radio`
and `button`.

**Three of its controls are wired, and two say why they are not.** View mode
(`Watch only`), the side zone (show/hide, dock left/right, width in 20px steps
with a reset) and Motion are live. The **geographic grid** and the **text-size
scale** are absent by name: the grid needs the map-grid slice — its projection
is `map_render`'s, and the shell cannot name a module that carries MapLibre —
and the text scale needs a global multiplier the shell's fixed sizes do not
have. The panel says so in one sentence rather than showing a control that
does nothing, which is the same rule the ADRs keep about not drawing a state
you do not have.

**The dock needed the column to follow the band.** Hiding the zone is one gate
(a cull in `live`, so the islands leave render AND ownership together), but
docking exposed a real bug: the islands' positions are stored, computed once
from the initial right-docked band, so moving the band left left them clipped
off it. They are now dock-bound — `pos.0 = band.0` every frame, with the drag
owning y — which is the console's arrangement too: islands ride the zone, they
do not float free. `view::zone_band_at` is the dock-aware band.

### Verification

| claim | how it was checked |
| --- | --- |
| the panel is the console's | capture: Settings, View/Layout/Motion, Watch only off, Side zone on, Docked right, Width **360**, Reduce motion off |
| the width stepper is wired | a click reached the `+` listener and drove `zone_w` to its clamp (360 → 520), which is also how the stray-click showed the control is live |
| the dock effect works | with the default temporarily docked left, the capture shows the island column at the LEFT edge (and without the `pos.0` fix, the capture showed it clipped away entirely) |
| the shell stays light | `proto/gpui-island` builds; `view` gained one function, nothing heavy |

**Verification.** `cargo test --lib` 421 pass (same 4 pre-existing failures);
both binaries and `proto/gpui-island` build clean. `TFG_SETTINGS=1` opens the
panel for a headless capture, the same seed-the-state precedent as `TFG_SEL`.

**Next.** The session: `users_game`, participants, groups — the plumbing that
turns `comms`, `dashboard` and `orders` from unportable into portable. That is
the next batch, and then those panels follow one at a time.

### U8 step 4 continued — the session, and the top zone (2026-10-13, fast-tracked)

The GPUI binary now holds a **session**, and the shell draws the **top zone** —
the first panel fed by the game rather than the register. This is the
prerequisite the last handoff named: `comms`, `dashboard` and `orders` all read
the session, and none could be ported before it existed.

**The read path, and why it stops where it does.** After login the binary calls
`games_list`, holds the newest game, and reads `game_participants` for its room.
That is the whole session: a `SessionView` of the game's name and state and the
room's counts. It deliberately does NOT join. `join_game` needs a **room key**
and `set_readiness` is a WRITE, and neither is a prototype's business; more to
the point, a seat is what the *dashboard* needs and the dashboard is not here
yet. The counts are `game_participants`' own — seat count, how many presented
the key, how many are ready — which is exactly what the top zone shows.

**The top zone is the console's, cut to what is sourceable.** Link dot and word,
the headcount, the held game and its state. The console's bar also carries map
controls, the clock and sign-out; those are their own slices, and the panel's
doc says so. The link dot is honest about its limits: the shell sees liveness
only as live markers, so a quiet wire and an empty map look the same — which is
why the word is `idle`, never `faulted`. The shell has not been told that
difference, and guessing it is the lie the console's link states exist to
avoid.

**One API fact the capture exposed.** The room count reads **0 of 2** — and that
is faithful: `/games/{id}/participants` sends `ready_at`, not `joined_at`, and
both this parser and the console's read `joined_at`, so neither can see who is
in the room on this server. The count is shown against the seats (`0/2`) so the
bar still says something true rather than a bare zero, and the gap is a server
contract, not a port.

### Verification

| claim | how it was checked |
| --- | --- |
| a real session is held | capture: `Game 15 (4x) 2026-10-08 19:50 · execution` — game 24 from the dev `/games` |
| the counts are the room's | same capture: `0/2 in the room · 1 ready`, matching the API's two participants and its one `is_ready` |
| the bar reads the wire | `live` with the live markers present |
| the session is read, not written | the binary calls only `games_list` + `game_participants`; no join, no readiness write |

**Verification.** `cargo test --lib` 421 pass (same 4 pre-existing failures);
both binaries and `proto/gpui-island` build clean.

**Next.** The seat: `join_game` (or a read of the caller's own participant row)
plus `game_units_list`, giving `auth_user_id`, the commanded hulls and the
groups. That turns `dashboard` — own ship, task group, helm — from unportable
into portable, and `orders` and `comms` follow it.

### U8 step 4 continued — the seat, and the People panel (2026-10-13, fast-tracked)

The GPUI binary now holds a **seat**, and the shell draws the **session
roster** — who is in the room, their seat, their readiness, with the caller's
own row marked. It is the hull register's twin, and the distinction is the
point: the register is what the fleet HAS, the roster is who is RUNNING it.

**The seat is READ, not joined — and that was the batch's one real decision.**
The console joins a game (a room key) and reads its seat from the answer.
That is a WRITE that seats the account in a live exercise, which is not a
prototype's business and not something to do by accident on a production game.
So `MinosMaster::me()` reads `/users/me` for the account's id, and the binary
looks for its own row in each game's roster instead. Same seat, no write. A
game whose participants are unreadable (403) is skipped — a room this account
cannot read is not a room it holds.

**The held game changed, and it should have.** Holding "the newest game" named
game 24, where this account has no seat. Holding "the newest game this account
is seated in" names game id 19 — which is *named* "Game 14" (`/games` and the
roster disagree about which number means what, and the roster is right). That
is the console's arrangement: the operator holds the game they are IN, not the
newest one on the server.

**Why this panel rather than the dashboard.** The plan was `dashboard`, and the
data says no: this account is a **Penyelenggara** — a judge seat — and commands
**zero hulls** in every game it is seated in. `dashboard_data` returns `None`
without a commanded hull, in the console too, so a dashboard here would be an
honest empty box. The roster works for every seat, which makes it the right
next panel; the dashboard waits for an account that commands something.

### Verification

| claim | how it was checked |
| --- | --- |
| the seat is read, not written | the binary calls only `/users/me`, `/games` and `/games/{id}/participants`; no join, no readiness write |
| the held game is the seated one | capture: `Game 14 · 3 seat(s) · 2 ready · you: Penyelenggara` — game id 19, where this account's row exists |
| the roster is the room's | the panel lists exactly the API's three seats (BUDI/Commando/ready, MARKUS/Commando/ready, SUPERUSER/Penyelenggara/judge) |
| the caller's row is marked | SUPERUSER's row is the SIGNAL-tinted one, with `you: Penyelenggara` in the header |

**Verification.** `cargo test --lib` 421 pass (same 4 pre-existing failures);
both binaries and `proto/gpui-island` build clean. `TFG_PEOPLE=1` opens the
panel for a headless capture.

**Next.** An account that commands something is what `dashboard` needs, and
`orders` needs one too — both are commander's panels. Two ways forward: seat a
test account on a hull in a dev game (a data change, not code), or port
`comms`, which works for any seat and whose gap is now measured (game 19's
inbox is reachable but empty, and games this account is not seated in answer
403). `comms` is the one that needs no new data.

### U8 step 4 continued — the commander's seat, and "My command" (2026-10-13, fast-tracked)

The dashboard is portable now, and this is the first batch that began with a
**data change** rather than code. The console's `dashboard_island` — "My
command" — is the seat, the hulls it commands, and their live fixes. Two of
its three columns are here; the third (task group) needs the session
hierarchy and the helm is a write, and the panel names both.

**The data change, and why it was needed.** This account was a
**Penyelenggara** — a judge seat — in every game it held, and two independent
walls said a judge cannot be a commander: `player_station()` requires a
non-judge seat, and the server refuses an assignment with "a judge-side
participant does not command units or task groups". So the seat was changed in
game 12 ("G10", **preparation** — the one state that is both dashboard-eligible
and writable) to **Commando** (role 109), and a hull assigned. Both are dev
data, and both are recorded here because they are state, not code.

**Two server constraints the change surfaced.** (1) **One person commands one
thing**: assigning a second hull answers "That person already commands the
unit …" — so giving this seat the hull that is actually ON THE WIRE (unit 8)
meant removing the first assignment, not adding to it. (2) The hold prefers a
**player** seat: the first version took the newest game this account was seated
in, which was an *execution* game where it is a judge, so the panel drew "no
hull" while a commander seat sat one game further down. The scan now prefers a
non-judge seat, then any seat, then the newest readable game.

**The fix is joined, not fetched separately.** "My command" finds each
commanded hull's course and speed in the LIVE markers **by unit id** — the same
string the roster and the wire share — so the panel never reads the registry,
and `LiveMarker` grew `heading_deg`/`speed_kn` to carry them. A hull with no
live marker reads `—` in every fix column: absent is not zero, and the panel
says so rather than showing `live` for a hull that has never reported.

### Verification

| claim | how it was checked |
| --- | --- |
| the seat is real | capture: `Commando · not ready to start` — the role written to game 12, and its readiness (cleared by the role change, as the server documents) |
| the hull is the seat's | same capture: `KRI Abdul Halim Perdanakusuma (KRI-355)` — game 12's unit 8, whose `id_commander` is this account |
| the fix joins the wire | same capture: `255° · 0.1 kn · stale` — unit 8's own last fix, ambered because it has gone stale |
| the shell stays light | `LiveMarker` gained two numbers; no new module, no store in the shell |

**Verification.** `cargo test --lib` 421 pass (same 4 pre-existing failures);
both binaries and `proto/gpui-island` build clean. `TFG_COMMAND=1` opens the
panel for a headless capture.

**Next.** The task group needs the session hierarchy (`groups` — three columns
in the console, none read yet), and the helm is the first panel action that
WRITES (a helm order). After those, `comms` and `orders` — and `orders` is
mostly helm, so the write path is the next real decision, not the next panel.

### U8 step 4 continued — the helm, the first write (2026-10-13, fast-tracked)

The console's panel actions have all been reads. "My command" now sends a
**helm order** — course and speed to a commanded hull — which is the first
thing the shell writes, and the first time the seam runs the other way.

**The seam, reversed.** Every feed so far is the binary handing the shell
data. The helm is a `Sender<HelmOrder>` the shell writes to and the binary
drains, with the server's answer coming back on a status feed of the same
shape as the others. The shell never learns a base URL, and a shell that
cannot reach the network cannot leak the token either.

**The rule is checked before the write, not after the refusal.** The helm arms
only in an executing game with a commanded hull — the console's own rule — and
that check lives in ONE method (`send_helm`) that both the button and the tests
call. The server enforces the same rule with a 409 ("Orders are given while a
game is in `execution`"), which is why the check is local: a disabled look that
still posts is the one thing this must not be.

### Verification

| claim | how it was checked |
| --- | --- |
| the order carries the right thing | `the_helm_sends_the_drafted_order`: the sink receives hull `8` with course 90 and speed 12 |
| it is silent when it must be | `the_helm_is_silent_outside_execution`: preparation puts NOTHING on the sink |
| the server's rule matches | driving the API: an order to a preparation game answers 409 naming the execution rule; an order to an execution game this account does not command it in answers the same |

**Verification.** `cargo test --lib` 423 pass (2 new; same 4 pre-existing
failures); both binaries and `proto/gpui-island` build clean.

**The live order is NOT demonstrated, and the reason is the environment, not
the code.** No dev game can be put in the one state the helm needs — executing,
with this account commanding a hull:
- **Execution games freeze setup**: seating and unit assignment answer 409,
  "a game's set-up may only be changed while it is in `planning` or
  `preparation`".
- **Preparation games freeze terms**: the planned window has no ends, and
  patching it answers 409, "a game's terms may only be changed while it is in
  `planning`" — so the game can never satisfy the start check, which is what
  the transition refuses on.
- **A fresh planning game is transiently the answer**: it was created, seated,
  given a hull, and its window set — but `POST /games/{id}/transitions` answers
  **403** for this account from *planning*, so it could not be started either.
  It was discarded (204) once the attempt was recorded.

So the write path is built and its contract pinned, and the one thing missing
is an environment where the server would accept it. That is the honest state:
`orders` and the rest of the helm's UI can be built now, but nothing should be
reported as *working against the server* until a game can be started.

**One dev-data change is left standing**: game 12 ("G10", preparation) holds
this account as a **Commando** commanding unit 8, which is what "My command"
draws and what the helm would order. It is left because removing it would
un-demonstrate the panel; it is recorded because it is state, not code.

### U8 step 4 continued — the write probe, and Comms (2026-10-13, fast-tracked)

Two things: a time-boxed attempt to unblock the helm's live verification, and
the next real panel.

**The write probe failed, and the answer is worth having.** The hypothesis was
that the transition's 403 was a SEAT problem — that a GM seat could start a
game a player could then be commanded in. It is not: a fresh game was created,
superuser seated as **Controller** (judge-side), the window set, and the
transition answered **403** from that seat too, exactly as it had from the
Commando seat. So the transition permission is not the game role; it is a
capability this account does not hold at all. Together with the three rules
already measured — orders need execution, messages need execution, and setup is
frozen once executing — the matrix closes: **no account available here can put
a game into the one state the write path needs.** The probe game was discarded
(204). This is recorded so it is not re-litigated: the helm's live verification
needs an account with the transition capability, and that is an environment
question, not a code one.

**Comms is the next panel, and it is a read.** The console's `comms_body` and
`messages_island` are, at their core, the session inbox; composing is a write
and IS blocked by the rule above, so the panel carries the read half and names
the other one in a sentence. `TFG_GAME` was added with it: pinning the held
game is a capture aid, and it is also the only way to photograph a panel whose
data lives in a game this account is not seated in — the inbox here is game
19's, while "My command" holds game 12.

**Two API facts the seeding found.** (1) `kind` has exactly two values —
`telegram` and `administrative` — and a **telegram from a judge must be sent
`AS` a scenario role**, so it needs the scenario composer's roles to exist
first; `administrative` sends without one, which is what the seeded message
uses. (2) The message's classification is **TERBUKA / TERBATAS / RAHASIA**
(open/limited/secret) and its degree comes from the mirrored
`message_degrees` helper — both real vocabularies the panel now displays.

### Verification

| claim | how it was checked |
| --- | --- |
| the transition is not seat-gated | a Controller seat answered the same 403 as a Commando seat — recorded, game discarded |
| the panel reads the real inbox | capture: `Game 14 · 1 message(s)`, sender `Super User`, class `Terbuka`, kind `administrative · broadcast`, the server's timestamp and the message body |
| the message is real | it was created by driving the API (`POST /games/19/messages`), and the panel shows the server's own fields, not a fixture |

**Verification.** `cargo test --lib` 423 pass (same 4 pre-existing failures);
both binaries and `proto/gpui-island` build clean. `TFG_COMMS=1` opens the
panel, `TFG_GAME=<id>` pins the held game, both headless-capture seeds.

**Next.** `orders` (mostly helm, resting on the unproven write — buildable, not
provable here), the task group (needs the session hierarchy, `groups`), and the
top zone's clock and map controls. And the standing question the write probe
leaves open: whether anyone can start a game on this environment at all.

### U8 step 4 continued — the camera, and the map controls (2026-10-13, fast-tracked)

The map has been a picture: every frame the shell drew came from the map
thread at ONE centre and ONE zoom, and the markers were projected at that same
constant. The top zone now carries the console's **map controls** (`−  z11  +`)
and a step actually moves the camera — the first time the picture is the
operator's rather than the build's.

**One camera, three readers, and that is the point.** A zoom is an `Arc<Mutex>`
the binary owns: the frame feed asks the map thread to render at it, the fix
feed projects the markers at it, and the camera thread writes it. Because all
three read the SAME value, the picture and the markers cannot decouple — a zoom
that moved only the map would slide every hull off its own position, which is
the bug this shape exists to prevent.

**The shell asks; it never transforms.** `zoom_by` clamps exactly as the
console clamps (`view::ZOOM_MIN`/`ZOOM_MAX`, 3..18), updates the readout, and
sends the value — the shell never scales the picture itself. The round trip is
the map thread's, and the readout is the number the shell ASKED for, which is
the console's own arrangement. The sink that carries it is now an enum
(`ToBinary`: helm or zoom), because there is one seam and one channel, and two
parallel channels would be two chances to forget one.

### Verification

| claim | how it was checked |
| --- | --- |
| a step asks the binary | `a_zoom_step_asks_the_binary`: `zoom_by(+1)` puts `ToBinary::Zoom(12.0)` on the sink |
| the shell does not transform | the test asserts the SINK, and `zoom_by` has no other effect than the readout |
| the camera reaches the map thread | with the opening zoom temporarily 13, the map area differs from the z11 capture by RMSE **0.538** — the map re-rendered, and the readout read `z13` |
| the map thread honours it | `shell_threads` re-creates the scene and calls `scene.set_center(at, zoom)` on a request |
| the markers follow | by construction: the fix feed projects at the same `Arc<Mutex>`, so the frame and the hulls share one camera |

**Verification.** `cargo test --lib` 424 pass (1 new; same 4 pre-existing
failures); both binaries and `proto/gpui-island` build clean.

**What this does not do.** The camera has no PAN, so the centre is still the
map's constant — and still sits under the right-docked chrome, which is why the
"centre a hull and it hides behind the panel" note from three batches ago still
stands. Panning is the obvious next step inside the camera, and it is the shape
the drag handlers already use.

**Next.** Pan (completes the camera), the task group (needs the session
hierarchy, `groups`), the clock (needs the Minos clock channel or a projection
from the game's window and `time_factor`), and `orders` — which stays
buildable-but-unprovable until somebody can start a game.

### U8 step 4 continued — pan (2026-10-13, fast-tracked)

The camera could zoom but not move, so the centre was the map's constant — and
still sat under the right-docked chrome, which is the note that has stood since
the markers went live. It can move now.

**The map surface is the pan handle, and painting first is what makes it the
right one.** GPUI hits the deepest element first, so a press over an island, a
panel, the top zone or a modal never reaches the map — panning is exactly the
gestures on bare map, with no exclusion list to keep in step with the chrome
as panels are added. The alternative (a guard in the root handler, testing the
pointer against the band and every island rect) would have to be rewritten
every time a panel appeared.

**The sign is a kernel function, not arithmetic in a thread.** `pan_center`
takes the drag in MAP pixels and answers the new centre: a drag points at the
map pixel under the cursor and asks for THAT GROUND to stay there, so the
centre becomes that pixel. `a_pan_drag_pulls_the_ground_with_the_cursor`
pins BOTH axes — dragging right moves the centre west, dragging down moves it
north — because the sign flip is per-axis and one of them inverted is the
classic bug. The shell sends MAP-pixel deltas (it is the side that knows both
the viewport and the frame's size); the sign and the unprojection are the
binary's, because the shell has no mercator.

**The camera is now (lat, lon, zoom), still shared.** Frame feed, fix feed and
the camera thread all read the same triple, so the picture and the markers move
together — the property that made the zoom safe applies unchanged to pan.

### Verification

| claim | how it was checked |
| --- | --- |
| a drag asks for a pan | `a_pan_asks_the_binary`: `pan_by(12, -7)` puts `ToBinary::Pan { dx: 12, dy: -7 }` on the sink, and a zero-delta move asks for NOTHING | 
| the sign is right on both axes | `a_pan_drag_pulls_the_ground_with_the_cursor`: right→west, down→north, and a no-op drag is no move (to 1e-9; the mercator round trip is float, not exact) |
| the centre reaches the map | with the camera temporarily pinned over unit 8, the map renders that area AND the marker sits at its projected position — one camera, both readers |
| panning cannot steal chrome gestures | by construction: the handler is on the element painted first, so deeper elements win the hit |

**Verification.** `cargo test --lib` 426 pass (2 new; same 4 pre-existing
failures); both binaries and `proto/gpui-island` build clean. `map_render`
gained `pan_center` and its test; `MAP_CENTER` is untouched.

**Next.** The clock (needs the Minos clock channel, or a projection from the
game's window and `time_factor`), the task group (needs the session hierarchy,
`groups`), and `orders` — still buildable-but-unprovable until somebody can
start a game.

### U8 step 4 continued — the Step panel, and a capture that hung (2026-10-13, fast-tracked)

**The Step panel** is the console's spine made visible: the exercise's running
step, the readiness gate, and the SERVER's assumed time. It is read-only on
purpose — declaring ready and advancing are writes, and the server takes both
only in the states this environment cannot reach — so the panel is what the
server will tell anyone with read on the game.

**Why the time shown is the server's.** `GET /games/{id}/current-step` answers
`assumed_time`, and the backend's own doc is unambiguous: "Never recompute
assumed-now client-side: the integral needs every segment plus the anchor,
which is three chances to disagree about what time it is in the exercise."
There is also no clock GET at all — the pause/resume/factor writes ARE the
clock read. So the panel displays the server's stamp and says whose it is,
rather than integrating anything locally. `None` is a real state (game 12
answers 409, "this game's scenario clock has no anchor"), and the panel says
so instead of drawing an empty step.

### The capture that hung, and what it cost

A capture command launched the binary, slept, and called `grim` — with no
timeout. **grim blocked at the Wayland screencopy layer** and wrote nothing.
Every later step therefore never ran, *including the `pkill` that was supposed
to reap the GUI process*, so the app stayed alive (its log shows 287 wire
reconnects over that stretch) and the command only ended when the harness
aborted it. Thirty-odd minutes of wall clock for a twenty-second job.

**Both faults were mine, and they are the same fault twice**: no `timeout` on
any external call, and cleanup that only runs if everything before it
succeeds. Every capture this session had that shape; this is the one that
happened to hang.

**`scripts/gpui-capture.sh` is rewritten around three rules**, each the fix for
a piece of it:

1. **Every external call has a `timeout`** — a blocked tool is a fast, named
   error, never an unbounded wait.
2. **The app is killed by a `trap`**, not by a statement at the end — a failure
   anywhere still reaps it.
3. **grim is PROBED before the app starts** — if screencopy is stuck there is
   nothing to capture, and the honest answer is an eight-second refusal that
   says so and leaves nothing running.

The probe earned its place immediately: grim was stuck when the new script was
written and worked again minutes later, so the blockage was transient. The
capture below was taken with the hardened script, which also proved the trap by
leaving no process behind.

### Verification

| claim | how it was checked |
| --- | --- |
| the panel reads the server's step | capture: `assumed 2000-01-01 19:00:00+07:00` — game 19's `current-step`, with the line saying whose time it is |
| absent halves stay absent | same capture: `nothing playing`, `no step asks you` — the server sent neither, and the panel does not invent them |
| a stuck capture now fails fast | the probe refused in 8s while grim was blocked; with grim back, the same script captured in 16s and left no process |

**Verification.** `cargo test --lib` 426 pass (same 4 pre-existing failures);
both binaries and `proto/gpui-island` build clean. `TFG_STEP=1` opens the
panel, `TFG_GAME=<id>` pins the game, both headless-capture seeds.

### U8 step 4 continued — the Inspector (2026-10-13, fast-tracked)

The Inspector: the SELECTED hull's detail, docked along the bottom — identity,
position, course, speed, trail and state, in one row. It is the second half of
the selection loop the roster opened (the sheet chooses, the dock answers), and
it is the first panel that needed **no new plumbing at all**.

**A dock, not a window.** The console recomputes the Inspector's place every
frame so a drag snaps back — a dock is a place, not a window that remembers —
and the shell does the same. It renders last of the chrome and under the modal
layer, because a dock is something the panels cover, not the reverse.

**Everything shown was already in the shell.** The roster carries the identity
(`name`, `hull`, `class`, `unit_id`), the live markers carry the fix. The only
kernel addition was three fields on `LiveMarker`: `latitude`, `longitude` and
`trail` (a COUNT — the shell has no use for the points and every reason not to
copy them per tick). A panel that costs three numbers is what a finished kernel
is supposed to feel like; the console's own Inspector reads the same two
sources plus the registry.

**The editors are named, not faked.** The console's three columns carry editors
for the map symbol, the affiliation and the heading. Those are writes, and the
same rule blocks them as blocks the helm, so the dock says so in a sentence —
the rule every panel here has followed rather than showing controls that do
nothing.

### Verification

| claim | how it was checked |
| --- | --- |
| it reads the selection | capture: `KRI Abdul Halim Perdanakusuma · KRI-355` — roster row 1, chosen by `TFG_SEL=1` |
| identity comes from the register | same capture: `class Ahmad Yani / Van Speijk · Frigate`, `unit id 8` |
| the fix comes from the wire | same capture: `position -7.8816, 112.5356 · 255° · 0.1 kn · 1 fix(es) · stale` — unit 8's own fix, ambered stale |
| no selection is a real state | the dock renders `no selection` rather than an empty row |

**Verification.** `cargo test --lib` 426 pass (same 4 pre-existing failures);
both binaries and `proto/gpui-island` build clean. `TFG_INSPECT=1` opens the
dock, `TFG_SEL=<row>` picks the selection, both headless-capture seeds — and
this capture went through the new `scripts/gpui-capture.sh`, which left no
process behind.

**Next.** The task group was the intended next column, and the data says no:
`/games/{id}/hierarchy` returns **empty** for every dev game, and
`HierarchyNode` carries no commander, so there is nothing to derive a task
group from. That needs a game with a real task organisation — the same class of
gap as the write probe. Otherwise: `orders` (unprovable until a game can be
started), the map's jump-to-unit, and sign-out.

### U8 step 4 continued — the sign-in card (2026-10-13, fast-tracked)

The shell logged in from `TFG_API_USER`/`TFG_API_PASS`. That was fine for
driving captures and useless for shipping, and it was the biggest single piece
of the console still missing. There is a **sign-in card** now, and it is the
first panel that has to exist before any other.

**Hand-rolled text fields, because GPUI has none.** GPUI 0.2.2 ships the input
PLUMBING — `Keystroke::key_char`, focus handles, `on_key_down` — and no text
field widget. So a credential field here is a focus handle, a keystroke handler
and a value: printable characters append, backspace removes, tab moves between
the fields, Enter submits. Deliberately no caret and no selection; a card is not
an editor. `key_char` and not `key`, because the key NAME is the legend on the
keycap and differs by layout while `key_char` is the character actually typed.

**One sign-in, one authenticated read.** The shell sends the credentials on the
sink the helm and the camera already use; the binary logs in and answers with
the register AND the session TOGETHER, because they are one authenticated read
and half a session is a state nothing should have to render. A refusal comes
back as a sentence the card shows — never a swallowed error — and the password
is cleared the moment it has been spent.

**Starting signed-out is the shipped state, not a test state.** `SessionView`'s
`Default` keeps `signed_in: true` for the tests and the parity proto, and only
the binary ever clears it: it is the one thing that knows whether it failed to
authenticate. With no credentials the shell now opens on the card rather than
on a console with an empty register and a fixture roster.

### A debugging pass the capture helper cost, and the rule that fixes it

The card was built, the tests passed, and three captures in a row showed the
CONSOLE. The shell had `signed_in = false` at construction and the render took
the CARD branch — confirmed by instrumenting both — so the picture was of a
DIFFERENT window: Hyprland places this window somewhere new each launch —
`(768,26)` and `(0,445)` a minute apart — and the helper matched by PID and a
rect read from a listing. It cropped whatever was there.

Two things came out of it. The window's **class is `tfg-gpui`** (the long-
standing "app_id mystery" resolved — it reports now), and the helper matches by
**class first, PID second**, which is immune to both the placement and the pid
reuse. That is rule 4 in the script's header, with the cost written down beside
it. The lesson is the ordinary one and it keeps being expensive: a capture that
shows something plausible is not a capture that shows the thing you changed.

**And the machine was mid-way through an `omarchy-flux` package update** while
all this ran — a terminal on screen doing an install — which is where the load
and very likely the grim hang came from. Worth knowing before reading anything
into a slow frame time from that window.

### Verification

| claim | how it was checked |
| --- | --- |
| the card renders when signed out | capture: `ARCONS · Command Center · Sign in`, the identifier field focused (cyan edge) and the password masked |
| typing is real | `the_sign_in_card_collects_what_is_typed`: type, backspace, retype, tab, Enter → `ToBinary::SignIn { "superr", "pw" }` on the sink |
| a half credential is refused locally | `the_sign_in_card_refuses_a_half_credential`: a status line and NOTHING on the sink |
| the signed-out state is reachable | `TFG_SIGNED_OUT=1` (a capture seed); with no credentials the same path is the shipped default |
| the capture is of the right window | by class now — the three wrong crops are why rule 4 exists |

**Verification.** `cargo test --lib` 428 pass (2 new; same 4 pre-existing
failures); both binaries and `proto/gpui-island` build clean.

### U8 step 4 continued — jump-to-unit, and sign-out (2026-10-13, fast-tracked)

The camera could zoom and pan but not go anywhere, and the top zone had no way
out. Both are closed now: the Inspector carries a **Centre** button that frames
the selected hull, and the top zone carries the console's **Sign out** at its
right end. Neither needed a new panel, a new feed, or a new seam — one `ToBinary`
variant each, on the channel that already exists.

**The jump is the console's `request_frame`, without the glide.** The shell names
the GROUND (lat/lon from the selected hull's own fix) and the chrome's shift in
px — it is the side that knows the zone's dock and width — and the binary
unprojects the visible centre, because only it has the mercator. The shift is a
kernel function, `view::camera_centre_shift`, mirroring
`chrome::camera_centre_offset_with` (half the zone width, sign by dock, zero
when hidden), pinned by `a_covered_zone_shifts_the_visible_centre_off_the_
window_middle`. A selection with no fix sends nothing: a hull that has never
reported has no ground, and framing `(0, 0)` would be a lie about where it is.

**Sign-out is local first, like the console's.** The shell tells the binary to
drop its token copy (so nothing else posts on it) and shows the card on the next
frame; the live markers go with the session. There is deliberately no server
revocation here: the binary never held the refresh token, and it keeps no
keyring — the console's `sign_out` revokes and clears both, and this prototype
has neither to clear.

**The same batch fixed a real token bug the sign-in card left behind.** The
card's sign-in logged in on the order thread but dropped the fresh token with
the answer, so the thread kept the STARTUP token — empty whenever the shell had
opened on the card — and the first helm order after signing in was refused. The
thread now keeps the fresh token and game on a successful sign-in. Found by
reading, not by a failure: nothing had signed in through the card and then
ordered yet.

### Verification

| claim | how it was checked |
| --- | --- |
| the jump asks for the hull's ground | `a_jump_asks_for_the_selected_hulls_ground`: Jump carries unit 1's fix lat/lon with shift **−180px** (right-docked 360px zone) |
| a fixless selection is silent | `a_jump_is_silent_without_a_fix`: no fix → `false`, nothing on the sink |
| sign-out clears both sides | `signing_out_clears_the_session_and_tells_the_binary`: `SignOut` on the sink, `signed_in` false, zero live markers |
| the shift mirrors the console | kernel test pins the fraction (0.5), both dock signs, and hidden-is-zero |
| the jump runs end to end | log: `centering on -7.8816, 112.5356 (shift -180px)` then `map frame 1001 (pump 6)` — unit 8's own fix, through the real click path (below) |
| the frame painted is the jump's | capture status line: `map raw frame 1001 ret 3 \| 1 live markers` |
| the buttons render | capture: `Sign out` at the bar's right end; Inspector `Centre` beside the close, enabled with the fix shown (`-7.8816, 112.5356 · 255° · 0.1 kn · stale`) |

**How the live jump was proven with no input synthesis.** The compositor
delivers the click that focuses the window INTO the client — the cursor sits on
the Centre button in both captures — so a stray focus click drove
`jump_to_selection` through GPUI's own event path: shell → sink → binary
unproject → map re-request → painted frame 1001. That is a genuine end-to-end
run, not a test double. It is also a capture hazard, twice observed: the same
mechanism opened the Comms modal unasked in the second capture, so a capture
can show a panel nobody seeded. Read the status line before reading the chrome.

**Verification.** `cargo test --lib` 432 pass (4 new: 1 kernel + 3 shell; same 4
pre-existing failures); `cargo test --bin tfg` 12 pass; both binaries and
`proto/gpui-island` build clean at the 8 pre-existing warnings, none new.

### U8 step 4 continued — the New-session composer (2026-10-13, fast-tracked)

The first panel that WRITES and can be proven here. Session creation is a
planning-state POST, and planning writes succeed on this environment (the
write probe proved games create, seat, assign and window — only transitions
answer 403). So the console's `session_creator_body` is ported whole: the
form, the validation, the POST, and holding the new game after it.

**The form is the console's, row for row.** Name (required) plus every
optional the `POST /games` body accepts — mode, description, purpose, target,
area, map tag — label above input in the console's web-form order, with the
console's own sentences (`name the session first`, `Name is required — the
server refuses a nameless session`). The mode rides radios, not a field,
because it is a choice of three with the console's one-line meanings. The six
text fields reuse the sign-in card's hand-rolled pattern (focus handle +
keystroke handler + value, tab walks all six, Enter submits) through indexed
accessors, so the rows cannot drift apart.

**The write runs the console's rule, twice.** The panel refuses a nameless
draft locally AND the binary refuses it again at the seam — the seam must not
trust the renderer. On success the new game is HELD, the way
`apply_created_game` holds it: `fetch_session` gained a `pinned` id (the
`TFG_GAME` env seed moved inside as the startup caller), the create arm reads
the game back pinned to the new id, and the shell replaces its session, clears
the form and closes. Born in planning by contract, read back rather than
assumed. A signed-out shell is refused at the seam, like the helm's backstop.

**One deliberate gap, stated on the panel's doc.** The console gates the door
on an app role (`can_manage_sessions`); the shell carries no roles yet, so the
door stands open while signed in and the server has the last word. Roles are
the next datum, not the next panel.

**The card is taller than any readout panel**, so the chamfered silhouette
grew a sized variant (`modal_silhouette_at`) — the console's own creator is
560x620, and shrinking this form onto the 480x320 readout card would ship a
cramped lie about the layout. Existing callers keep `modal_silhouette()`,
pixel-unchanged.

### Verification

| claim | how it was checked |
| --- | --- |
| the form is the console's | capture: `New session`, intro sentence, Name + hint focused (cyan edge), required note, 3 mode radios (maneuver lit), meaning line, 5 prose fields with the console's hints, Cancel + dimmed `Create session →` + `name required` |
| the `→` renders | visible on the Create button — no tofu, no fallback needed |
| a nameless draft never POSTs | `the_composer_refuses_a_nameless_session`: status set, sink empty |
| a named draft sends the whole form | `the_composer_sends_the_whole_draft`: all seven fields verbatim on the sink, busy latches |
| the answer holds the game | `applying_a_created_session_holds_it`: panel closes, form clears, mode resets, `planning` game held |
| the server takes the arm's params | drove the API: `POST /games` (name/mode + 5 prose rows) → **201 planning** (game 27), `DELETE` → 204 — created and discarded, no residue |
| the seed is action-free | `TFG_COMPOSE=1` opens the form with an EMPTY draft — a seeded name would be one stray click from a real game |

**Verification.** `cargo test --lib` 435 pass (3 new; same 4 pre-existing
failures); `cargo test --bin tfg` 12 pass; both binaries and
`proto/gpui-island` build clean at the 8 pre-existing warnings, none new.

### U8 step 4 continued — the Book panel (2026-10-13, fast-tracked)

The held game's scenarios with their steps (read), plus the new-scenario
title row (the one write). The second planning-write proven end to end after
session creation — and the one that corrected its own premise mid-slice.

**The read is the console's `book_view` without the staging.** Server
scenarios in book order, steps numbered with their windows, empty content
weak (`(not written yet)` is an authoring state, not an error). Staged cuts
hidden and staged adds appended have no meaning here, because this shell
stages nothing: the console authors through `book_draft` and syncs on
advance, and there is no advance to sync on — so adds POST at once, and the
panel states the difference in a sentence. Remove, relate and window editing
are the staging machinery's and are named there rather than faked. Steps
arrive ONLY nested in the scenarios (there is no step-list route), so the
panel reads `session.book` and nothing else, and a step has NO title — the
content is the label, which is the backend's decision.

**The 403 that shaped the write.** The first contract drive — scenario POST
into a fresh game this account created but was never seated in — answered
**403**. Seating the account as Penyelenggara (role 270) and retrying
answered **201**. So scenario authorship requires a SEAT, not just a token,
and the arm reports the server's refusal rather than guessing at it. No
auto-seat: seating chooses a role, and choosing roles by prototype is exactly
the accident the seat-is-read-not-joined rule exists to prevent.

### Verification

| claim | how it was checked |
| --- | --- |
| the book reads the server's | capture (`TFG_BOOK=1 TFG_GAME=19`): `Game 14 · 1 scenario(s)`, `Scenario 1 - 2 step(s)`, `1 Do A then B 1000–1030`, `2 Do B again then C 1030–1100` — the API's own rows |
| a titleless row never POSTs | `the_book_refuses_a_titleless_scenario`: status set, sink empty |
| a titled row sends trimmed | `the_book_sends_the_title`: `AddScenario { "River crossing" }`, busy latches |
| the answer stays open | `applying_a_refreshed_book_holds_it_and_stays_open`: re-read book held, title cleared, panel open (the composer closes; this must not) |
| the server takes the arm's path | probe game → seat as Penyelenggara → `POST scenarios` → **201** (`River crossing`, id 7) → book lists it → game discarded (204), no residue |
| authorship needs a seat | unseated creator POST → 403; seated retry → 201 — recorded so it is not re-litigated |

**Verification.** `cargo test --lib` 438 pass (3 new; same 4 pre-existing
failures); `cargo test --bin tfg` 12 pass; both binaries and
`proto/gpui-island` build clean at the 8 pre-existing warnings, none new.

### U8 step 4 continued — book steps: add and remove (2026-10-13, fast-tracked)

The book panel grows its working half: the open scenario's steps with
per-step remove, and the new-step row (content plus window, both ends or
neither). Steps still post at once — no staging, stated on the panel — and
the read they return to is the same re-read book.

**The draft is the console's own.** The step row is held as a
`book::ComposerDraft` — content plus HHMM window with its `problem()` rule —
not a copy of it, because a duplicated validation is a second answer waiting
to disagree. Empty content is LEGAL (an unwritten step is an authoring state,
drawn weak); only the window rule can refuse. The open scenario defaults to
the book's first, the console's landing rule; tapping a title opens it.

**The move this slice forced.** The shell names `ComposerDraft`, and
`problem()` names `crate::backend::hhmm_*` — which would drag the HTTP stack
behind the parity proto's fast loop, the exact thing the proto exists to
avoid. So `hhmm_ok`/`hhmm_window_ok` moved to `view`, verbatim, with
`backend` re-exporting both names: every existing path (master's internals,
its tests, the binary arm's guards) resolves unchanged, and the proto shims
`book` like `view`. Pure string validation was kernel all along; it just
lived in the wrong module.

**Three API facts the probe paid for.** (1) Scenario POSTs need a SEAT —
an unseated creator gets 403, seating as Penyelenggara gets 201 (last
slice's finding, re-confirmed). (2) Game role ids differ PER GAME
(Penyelenggara was 270, 288 and 297 on three fresh games), and call signs
are globally unique across live games (`SUPERUSER` is taken by the live seat
in game 19) — the 409s they caused are recorded so nobody re-debugs them.
(3) A content-only step PATCH NULLS the window: absent does not leave alone,
whatever the client doc says. Window editing therefore resends the window
explicitly — which is why it is deferred, not built on a guess.

### Verification

| claim | how it was checked |
| --- | --- |
| empty content sends, half window does not | `empty_step_content_is_legal_but_a_half_window_is_not`: empty posts to scenario 4; `1000` + empty end refused, busy never latches |
| bad windows never reach the wire | `a_bad_window_never_reaches_the_wire`: `10-1030` and `1100-1000` both refused, sink empty |
| the whole row sends | `the_step_row_sends_content_and_window`: content + `(1000, 1030)`, busy latches |
| remove names its step | `removing_a_step_asks_the_binary`: `RemoveStep { 4, 7 }` |
| the step lifecycle works | probe game → seat → scenario → add step **201** (window stored) → PATCH content-only **200** (window nulled — the quirk) → DELETE step **200** → book empty → game discarded (204) |
| the panel holds the working half | capture (`TFG_BOOK=1 TFG_GAME=19`): open title cyan, 2 steps with remove buttons, content + window row, Add step, title row below — all on one card |
| the move changed nothing | `hhmm_*` tests green through the re-export; proto builds with the `book` shim |

**A capture correcting the layout.** The first capture put the book on the
readout card and clipped the title row off its bottom — the form had
outgrown the card the moment steps arrived. The book now shares the
authoring card (`FORM_W/H`, renamed from `COMPOSE_*`), and the re-capture
shows the whole form. Readout cards are for readouts.

**Verification.** `cargo test --lib` 442 pass (4 new; same 4 pre-existing
failures); `cargo test --bin tfg` 12 pass; both binaries and
`proto/gpui-island` build clean at the 8 pre-existing warnings, none new.

### U8 step 4 continued — readiness: declare and withdraw (2026-10-13, fast-tracked)

The seat's own readiness gate, in "My command": declare as a player,
withdraw when ready, nothing for judges (exempt) and the seatless. The panel
that corrects the handoff's blocked-write section — readiness was listed as
refused and had simply never been probed as the seat that can declare.

**The probe.** PUT `/games/12/readiness` as the Commando seat in preparation
answered **200** with `is_ready` and a `ready_at` stamp; DELETE answered 200
with `False`. Prior state (not ready — the role change cleared it) restored,
no residue. Planning-state writes now stand at six proven: session create,
scenario add, step add, step delete, readiness declare, readiness withdraw.

**The row is the console's `readiness_ui`, all four states.** Judge → the
exempt sentence; ready → name, role and a withdraw button; player → name,
role and the declare button; no seat → the room-key sentence. The rule lives
in one method (`set_readiness`) that both buttons and the tests call — no
seat, judge, asking for the held state, or an in-flight ask sends anything.
The answer re-reads the session pinned to the held game (the hold follows
the answer, the console's `apply_join` rule in miniature) and the panel stays
open. The helm below it stays disarmed: readiness does not wait for
execution, which is exactly why this row is live while the helm is not.

### Verification

| claim | how it was checked |
| --- | --- |
| a player declares and withdraws | `readiness_declares_and_withdraws`: `SetReadiness { true }` then `{ false }` on the sink, busy latches between |
| judges and the seatless are silent | `readiness_is_silent_for_judges_and_the_seatless`: both directions, both judge states, plus no seat — sink empty throughout |
| the held state is a no-op | `readiness_asks_for_nothing_already_held`: declaring while ready sends nothing |
| the answer is held, panel open | `applying_a_readiness_answer_holds_the_session`: re-read readiness kept, latch cleared |
| the server takes it | PUT → 200 (`is_ready`, stamped) → DELETE → 200 (`False`) on game 12, state restored |
| the row renders | capture (`TFG_COMMAND=1`): `Commando · not ready to start`, `Super User · Commando` + `ready to start` button above the hull and the disarmed helm |

**Verification.** `cargo test --lib` 446 pass (4 new; same 4 pre-existing
failures); `cargo test --bin tfg` 12 pass; both binaries and
`proto/gpui-island` build clean at the 8 pre-existing warnings, none new.

### U8 step 4 continued — relate: what a step obliges (2026-10-13, fast-tracked)

The book's last working half: per-step relate counts, and the relate editor
under its step — hull checkboxes over the exercise's pieces, the nodes
section honestly empty, save posting the working set whole. The slice that
also closed the book: the two remaining "remainder" items have no console
surface at all (below).

**The shape.** The server resolves each step's list (posts + obliged
personnel), so the session read fans out one `step_related` GET per step —
best-effort per step, one unreadable list never sinks the session — and the
editor redraws from the read, never from what was sent. The working set seeds
from the read when the editor opens (there is no staging here, so the working
set IS the last saved list), flips one ref per tap, and saves whole — empty
included, which is legal and clears. Counts (`{n} post(s)`) ride every step
row; absent reads claim no count rather than zero. The pieces are assigned
units only: the server 400s any other hull, so the whole register would be a
menu of refusals. Nodes read "no task organisation loaded" — the hierarchy is
empty in every dev game, the console's own empty-state.

**The probe behind it.** Relate hull 8 (assigned) → **200** with resolved
names and obliged personnel → read-back identical → clear (empty list) 200 →
game discarded (204). Relate unassigned → 400 ("has to be one of its
pieces"); judge-side seat commanding → 400 (command needs a player seat, so
the probe reseated Commando and assigned + related cleanly). Empty list is
legal — the console's "gate refuses an empty list" is about the step gate,
not the PUT.

### Verification

| claim | how it was checked |
| --- | --- |
| the editor seeds from the read | `relate_opens_seeded_from_the_read`: open → working is the read's posts; toggle again → closed |
| one ref flips, rest stay | `relate_working_flips_one_ref`: add 9, drop 8 → only 9 |
| save posts whole, silence otherwise | `relate_saves_the_working_set_whole`: `SetRelated { 4, 7, [(hull, 8)] }`, busy latches; shut or in-flight saves nothing |
| the answer reseeds, stays open | `applying_a_relate_answer_reseeds_and_stays_open`: working becomes the resolved list |
| the panel shows the working half | capture (`TFG_BOOK=1 TFG_GAME=19 TFG_RELATE=1`): `0 post(s)` counts, open editor with `obliges nobody yet`, two piece checkboxes, empty nodes, save/close, step and title rows below |
| the fan-out reaches the shell | same capture: game 19's two pieces listed — the pieces read works on live data |

**Two non-items, recorded so they are not re-planned.** Window edit and
reorder were carried as "the book's remainder", and neither has a console
surface: no caller posts `update_scenario_step` (windows are set at add and
never edited), and nothing calls `reorder_scenarios` at all. There is nothing
to port — the book (shelf, scenario add, step add/remove, relate) is done.

**Verification.** `cargo test --lib` 450 pass (4 new; same 4 pre-existing
failures); `cargo test --bin tfg` 12 pass; both binaries and
`proto/gpui-island` build clean at the 8 pre-existing warnings, none new.

### Unblocking execution: measured, not built (2026-10-13)

**Transitions need the Penyelenggara seat.** Seating this account as
Controller, ADMIN TESTING, Deputy and Referee on fresh planning games all
answered **403**; seating as Penyelenggara answered **200** (`planning →
preparation`, no preconditions — the spec's table holds). The spec says it
plainly once you look: transitions are "authorised by being this game's Game
Master", and the 403 is the not-GM answer. The two earlier 403s (Commando,
Controller) were seats, not capabilities — the hypothesis is closed.

**One account cannot span the two halves.** A second seat for the same user
answers 409 ("One person holds one role per exercise"), and reseating clears
readiness (spec, re-confirmed by the role-change wipe on game 12). So the
loop is shut for a single account: the GM seat transitions but cannot
command or declare (judge-side), the player seat commands and declares but
cannot transition, and switching seats wipes the declaration the execution
gate needs. Execution takes two hands — or the Minos admin acting directly.

**Game 19 is already execution — and useless for the helm.** `Game 14`
executes with Budi/Markus Commando and this account judge-side commanding
nothing. Setup is frozen there (409), so no hull can be handed over. A fresh
game is the only path, and it needs the two-hand recipe in the next log's
answer to the owner.

### Execution unblocked, helm proven live (2026-10-13)

The owner supplied two more accounts (budi, markus), and the write half fell
in one session. The findings that did it, each measured:

- **Transitions need the Penyelenggara seat.** Controller, ADMIN TESTING,
  Deputy, Referee → 403; Penyelenggara → 200. The spec: transitions are
  "authorised by being this game's Game Master".
- **One person holds one role per exercise** (409 on a second seat), and
  reseating clears readiness — so a single account cannot span GM + player.
  Two accounts are the whole trick.
- **Readiness declares in preparation only** (planning PUT → 409 naming it).
- **Execution needs three things named at once:** the planned window
  complete (`actual_end` + `assumed_end`, each after its start — PATCHable in
  planning only, frozen after), every unit placed on the map (PUT placement
  works in preparation), and the usual units + players + all-ready. The 409
  names all of them together.
- **Field names matter:** the order route takes `heading_deg`/`speed_kn`
  (400 otherwise) — exactly what the binary's `order_unit` sends.

**Game 42 ("GPUI helm proof") executes.** Created planning → both seats →
unit 8 assigned + placed + window set (all planning) → GM to preparation →
superuser declares → GM to execution. It stands as the proof environment:
superuser Commando commanding placed unit 8, ready; budi Penyelenggara GM.

### Verification

| claim | how it was checked |
| --- | --- |
| the helm posts live | `POST /games/42/units/8/order` `{heading_deg: 90, speed_kn: 12}` → **201**, fix answer (`90°`, `12 kn`, unclamped, assumed-time stamped) — the binary's exact body |
| the shell holds the execution game | capture: `GPUI helm proof · execution`, `Commando · executing`, readiness row with withdraw, hull with live fix, helm ARMED (`SET HELM` full-strength, "An order posts at once…") |
| the contract matches the code | the 201 body is byte-for-byte what `order_unit` sends; the sink test pins the shell half |

**Messages: one level deeper, mapped but not taken.** Administrative is
judge↔GM traffic (player 403, "send a Telegram instead"); telegram needs
the hull under a task-organisation node ("no chain from it"), and nodes POST
only in planning/preparation — game 42's tree is fixed empty. A telegram
proof wants a fresh game with the hierarchy built before execution (node
create schema + hull→node placement route still to discover). Next slice's
problem, with the composer UI it belongs to.

**Standing dev-data, recorded.** Game 41 (preparation, the windowless
attempt) cannot be discarded — DELETE is planning-only (409) — so it stands
with our two seats. Game 42 executes and must stand. Both are state, not code.

**Verification.** No code changed this slice (setup only): `cargo test --lib`
446 pass still holds by inspection — nothing to re-run for a slice with no
diff. The helm's tests (`the_helm_sends_the_drafted_order`,
`the_helm_is_silent_outside_execution`) are now live-backed rather than
contract-hoped.

### Comms composer + telegram proven live (2026-10-13)

The owner supplied the CMS collection (`docs/minos-backend-postman.json`),
which named the two missing routes outright: `POST /games/:id/hierarchy`
(`{id_unit_hierarchy, name, ...}`) and `PUT
/games/:id/units/:unit_id/hierarchy` (`{id_hierarchy_node}`). The echelon
vocabulary at `GET /hierarchy` is four rows (Unsur, Satuan Tugas, Gugus,
Operasi Gabungan); a root node takes rank 4 with no parent.

**Game 43 ("GPUI telegram proof") executes with a task organisation.**
Planning: both seats (superuser Commando, budi Penyelenggara) → node
"GUGUS PROOF" created (201, id 1) → unit 8 assigned + map-placed + hierarchy-
placed (200, `id_hierarchy_node: 1`) → full window → GM to preparation →
superuser declares → GM to execution. It stands beside game 42 as proof
environment; game 41 (windowless preparation) still stands, undiscardable.

**Telegram contract, measured:** `POST /games/43/messages`
`{kind: telegram, classification: TERBUKA, id_degree: 1, content, broadcast:
true}` → **201** with the message answer. Administrative stays judge↔GM-only
(403); the addressing GET 400s for a commander ("speaks from the post they
hold") — correctly unused. The binary's `SendTelegram` arm sends byte-for-
byte this body (telegram/TERBUKA/degree 1, empty to/cc = broadcast).

**Shell:** comms panel gains the composer row — hand-rolled field (index 10,
tabs to itself, Enter sends), SEND armed only in execution, `send_telegram`
refusing non-execution and blank rows in the console's sentences, `ToBinary::
SendTelegram`, binary arm with re-read, `apply_message` staying open. The
panel states the cut (broadcast/TERBUKA/BIASA; kind, marking, audience,
grade pickers later; hull must sit under a task node).

### Verification

| claim | how it was checked |
| --- | --- |
| telegram posts live | curl 201 on game 43 with the binary's exact body |
| shell holds the thread | capture `/tmp/gpui_comms.png`: `GPUI telegram proof · execution`, live broadcast row (Terbuka, 09:30), `New telegram` field + armed SEND + cut aside |
| sink contract | `the_composer_sends_a_telegram_broadcast`, `the_composer_is_silent_until_execution_and_content` |
| no regressions | `cargo test --lib`: 452 pass, same 4 pre-existing failures (`book::prune…`, `map_render::quad_bow…`, `roster::seats…`, `sim::order_move…`); `cargo check --bin tfg_gpui` clean, no new warnings |

### Step declare proven live, GM verbs named-off (2026-10-13)

**Game 44 ("GPUI step proof") executes with a book that asks.** Planning:
both seats → node "GUGUS STEP" (id 2, global sequence) → unit 8 assigned +
map-placed + hierarchy-placed → scenario "First move" + step "Take station
east of the buoy" (1000–1100) → related `[{hull, 8}]` (200) → window → GM to
preparation → superuser declares → GM to execution. `GET current-step` as
superuser answers `asking`: scenario 12, step 16, required 1, outstanding 1,
`you.ready: false` via hull 8.

**Declare contract, measured:** PUT `.../steps/16/readiness` → **200**
(outstanding 0, `you_ready` true, gate `may_start: true`) → DELETE → **200**
(restored to outstanding 1, `you_ready` false). Game 44 stands pristine with
the ask live.

**Scope, decided from facts:** the console aims Play/Skip/Rewind from
`step_runs` tracked off the socket (`next_step`, `latest_terminal`), and
there is no runs GET — only `current-step`. So GM verbs are un-aimable in
this shell and render visible-but-off for every seat, the console's own
read-only-seat pattern, with the sentence saying why. The participant
declare — the write OUR seat owns — is fully built: `StepAsk`/`StepLine`
carry ids, `send_step_declare` refuses no-asking and stale steps,
`ToBinary::DeclareStep`, binary arm with re-read, `apply_step_declare`
staying open. GM verbs are the next slice and need socket step-run
tracking first.

### Verification

| claim | how it was checked |
| --- | --- |
| declare posts live | curl PUT 200 + DELETE 200 on game 44 with counts/gate/flag verified at each step |
| shell holds the ask | capture `/tmp/gpui_step.png`: `GPUI step proof · execution`, `Take station east of the buoy · First move · 0 of 1 ready`, armed DECLARE READY, off Play/Skip/End/Rewind + rule aside |
| sink contract | `the_step_row_declares_for_the_asking_step`, `the_step_row_is_silent_without_an_asking_step` (no-asking + stale-step silence) |
| no regressions | `cargo test --lib`: 454 pass, same 4 pre-existing failures; `cargo check --bin tfg_gpui` clean, no new warnings |

### Player step reminder strip (2026-10-13)

Per the owner's direction: the player UI is a reminder, not a list. A
one-line strip just under the top bar names the ONE step — the asking step
with scenario, instruction and counts, else what is playing — with an
indicator dot in the console's own colors (sand while the step waits on
this caller, signal green once declared or when only naming play). A click
opens the existing step modal, where the scenario and the declare row live;
the strip grows no panel of its own. Off outside execution and when there
is nothing to remind. The scaffold button row slides down by the strip's
height while it shows, so the reminder never covers it.

### Verification

| claim | how it was checked |
| --- | --- |
| strip shows the ask | capture `/tmp/gpui_reminder2.png`: amber dot + `NEXT · First move — Take station east of the buoy · 0 of 1 ready` under the top bar on game 44, modal closed, scaffold row slid clear |
| click opens the modal | first capture's modal was opened by a compositor focus click landing on the strip (modal defaults closed, proven by test + second capture) — the accidental live proof |
| composition logic | `the_reminder_names_the_asking_step`, `the_reminder_rests_once_declared_then_falls_back` (declared rest, asking-beats-playing, NOW fallback, silence cases), `opening_the_reminder_opens_the_step_modal` |
| no regressions | `cargo test --lib`: 457 pass, same 4 pre-existing failures; `cargo check --bin tfg_gpui` clean, no new warnings |

### The composed console: layout by state, and a left dock (2026-10-13)

The owner's direction was to stop shipping "buttons and islands floating on
the screen" and compose the console's real layout. This is that, in two
parts: the column's membership is now a function of where the exercise is,
and the zone docks LEFT like the console's.

**The scaffold is gone.** The nine-button strip (`OPEN ISLANDS`, `OPEN MODAL`,
`PEOPLE`, `MY COMMAND`, `COMMS`, `STEP`, `INSPECT`, `NEW SESSION`, `BOOK`) and
the three identical 360x300 demo islands with literal hull rows are both
deleted. Those buttons existed because the surfaces had no place: a demo's
answer to "where does this panel go" is a button that opens it. Now they have
places, and the band carries only the doors the console's band carries.

**The column is a stack, and its membership is the state's.** `view` gained
`IslandKey`, `IslandSlot`, `zone_islands(state, player)`, `zone_stack_height`,
`zone_slot_origin` and `zone_overflow` — the console's `side_zone` match, cut
to the surfaces this shell has. Island state is keyed by `IslandKey` rather
than by index, and the open/closed choice lives in the four `*_open` flags
with no second copy inside `IslandState`: two places holding "is this panel
shown" is two answers waiting to disagree, and the one that loses is always
the one the operator last touched. `column_slots()` is the ONE answer the
render, the wheel's chain and the reminder's reveal all read.

The islands no longer drag. That is not a simplification, it is the console's
arrangement: `zone_island_origins_with` gives every island its origin from the
stack, so the column is dock-bound and a dock change or a resize moves all of
it at once with no stored position to update. The grip is now the column's
only gesture on its geometry, and it straddles the zone's INNER edge —
`band.2` for a left dock, `band.0` for a right one, because putting it on the
outer edge parks it against the window border where there is nothing to drag
into.

| state | column |
| --- | --- |
| NoSession | Operator, Fleet |
| Planning | Operator, Fleet, Crew |
| Preparation | Operator, Command, Crew |
| Execution (organizer) | Operator, Step, Command, Crew |
| Execution (player station) | Operator, Step |
| Closure | Operator, Crew |

`player` is the console's `player_station()` VERBATIM — a seat held by a
non-judge — and deliberately does NOT also require a commanded hull. That is
the DASHBOARD's rule (`main.rs:12719`), and conflating them would hide the
organizer column from a seated player who has no hull yet, which is most of a
planning room. The shell's first cut required a hull and was wrong.

**Comms left the column.** It is a modal door now, like Settings and the
composer, opened from the band's comms door — the console's own flow, whose
inbox is a floating pane this shell has no machinery for. A 560pt slot in
every state's column would also have made the per-state layout a wall rather
than a composition. The band's door flips it, which is what a door does.

**One modal at a time.** `open_only(WhichPanel)` is enforced in one place
rather than trusted to each door, because a rule every caller must remember is
a rule that gets forgotten the moment a sixth door appears. The column's
islands are not in the rule and never were.

**The band is the console's row.** Groups separated by 1px rules rather than a
list of buttons, and a right-to-left group at the end holding comms, Settings
and sign out. The session readouts yield on a narrow window behind
`view::BAND_READOUT_BUDGET` (the console's `HINT_BUDGET`, same number, same
reason): the band is one row of readouts AND actions, and a left group that
eats the width pushes the right-hand group off the edge — taking sign-out with
it. The readouts yield; the doors never do. The map readout line moved behind
`TFG_READOUT=1`: it is genuinely useful for a self-describing capture and it
is also a developer's line floating over the map, which the console does not
have.

**The dock default is LEFT** (`main.rs:20669`). Three things followed it: the
band mirrors with the same gaps, the grip moves to the inner edge, and
`camera_centre_shift` answers positive for a left dock so a jump frames the
hull in the area the map actually shows. The jump test now pins BOTH signs,
because a sign that only worked on one dock is the classic one-axis-inverted
bug.

### Verification

| claim | how it was checked |
| --- | --- |
| the column follows the state | `the_column_follows_the_state_of_the_exercise`: all four states with no seat, then planning and execution with an exercise-side seat |
| the state table is the console's | `every_state_has_the_column_it_should`, `a_player_station_changes_the_column_only_in_execution`, `the_operator_island_leads_in_every_state`, `the_step_island_only_shows_in_execution` |
| a closed island stays closed | `a_closed_island_stays_closed_across_a_state_change`: shut Crew in planning, still shut in execution, in a different slot |
| a hidden zone holds nothing | `a_hidden_side_zone_holds_no_islands` |
| one modal at a time | `only_one_modal_opens_at_a_time`: each door closes the others |
| the dock default is left | `a_jump_asks_for_the_selected_hulls_ground` pins +180 left and −180 right |
| the column docks left and the grip is on its inner edge | capture `/tmp/gpui_clean.png`: OPERATOR and STEP islands at the left edge, the grip strip at their right, the map filling the rest with no inset |
| the band carries only doors | same capture: `live · + New session · − z11 + · comms · Settings · Sign out`, no scaffold row, no floating readout |
| the reminder still opens the step | `opening_the_reminder_opens_the_step_modal` now asserts the island opens AND asks to be revealed |

`cargo test --lib`: **472 pass**, same 4 pre-existing failures (`book`,
`map_render`, `roster`, `sim`). `cargo test --bin tfg` 12 pass. Both binaries
and `proto/gpui-island` build; the proto gained a `gamestate` shim because
`view` now names the state axis.

**A bug the capture caught, and it was not a stray click.** Three captures in
a row showed the comms inbox open over the console. The handoff's own hazard
note blames the compositor's focus click; this time the flag was simply
defaulted the wrong way — the island seeds had been inverted to
`!= "0"` (open by default, correct for a column island) and comms had become
a MODAL in the same slice, so it inherited the inversion and opened on every
launch. The rule the fix records: an inversion applied to one kind of flag
must not be applied to the other.

**A process failure, recorded because it cost real work.** Restoring
`view/mod.rs` was attempted with a slice-based `str.replace` whose start and
end indices resolved to an EMPTY match, and `"".replace("", x)` interleaves
`x` between every character: a 764-line file became 4.1 million. The file was
rebuilt from `HEAD` and the missing pieces re-derived from their source
modules — which surfaced that several earlier sessions' moves into `view`
(`hhmm_ok`, `MapSymbolShape`, `map_symbol_accent/fill`, `affiliation_*`, the
eight `FRAME_*`, `camera_centre_shift`, `box_holds_interior`, `ZOOM_*`,
`AUTH_FIELD_H`, `zone_band_at`) were UNCOMMITTED and would have been lost
with a `git checkout`. They are restored and now carried by the tests that
pin them. The rule: never compute a replace range from two `index()` calls
without asserting the slice is non-empty, and never assume a modified tracked
file's HEAD is a safe base.

**Still open, unchanged.** The map's centre is the window's, so a hull at the
centre sits behind the column — worse on a left dock, since the visible area
is now to the RIGHT of the middle. The selection is one-way (sheet → marker,
no marker → sheet). The affiliation frame, per-shape fill and the store
resolvers are wired but every hull still reads Unknown without a session.

### The column's wheel was dead code (2026-10-13)

The owner reported that the sidebar could not be scrolled. It could not, and
the reason is that **the column's wheel listener never ran at all** — not
"scrolled the wrong thing", never entered. `chained_wheel`, the ceiling, the
subtraction and the whole body-versus-column chain were unreachable code.

**The cause is one attribute.** GPUI's scroll dispatch gates every listener
through `HitboxId::should_handle_scroll`, which is
`window.mouse_hit_test.ids.contains(&self)`. That set is built by
`Window::hit_test`, which walks hitboxes newest-first and **`break`s on the
first `HitboxBehavior::BlockMouse`** — so every hitbox painted before it is
dropped, including the band's. The island's rim and its band texture were
`.occlude()`d, which is `BlockMouse`, and the rim covers the whole island. So
over any island the band's hitbox was never in the set, and
`Interactivity::on_scroll_wheel` (which wraps the listener in exactly that
check) never called the handler. GPUI's own doc on `occlude` says
`block_mouse_except_scroll` "should be preferred"; that is what
`BlockMouseExceptScroll` is for — it records `hover_hitbox_count` so
clicks and hover still cannot reach the map behind, and does not `break`.

**Why the tests missed it.** `view::tests` pins `chained_wheel` thoroughly, and
every one of those tests passed for the whole time the column could not
scroll — because they test the decision function, and the decision function was
never called. A kernel test cannot see a missing call site. The two new tests
dispatch a real `ScrollWheelEvent` through the shell, which is the only layer
that can tell:

- `a_wheel_over_the_column_moves_the_column` — wheel to the ceiling and back.
- `a_wheel_over_a_scrolling_body_leaves_the_column_alone` — the Fleet body
  takes it and `zone_scroll` stays 0. One gesture, one surface.

Two facts about driving this in the harness, both of which fail silently:
`should_handle_scroll` reads the hit test from the window's **mouse position**,
not from the scroll event's own `position`, so the pointer must be moved there
first or the listener is gated off; and the harness window is 1920x1080, where
a three-island column *fits* and the ceiling is 0, so a test must resize to a
window where it does not.

### Verification

| claim | how it was checked |
| --- | --- |
| the wheel reaches the column | `a_wheel_over_the_column_moves_the_column` (would have failed before the fix — the handler did not run) |
| the body still wins where it has room | `a_wheel_over_a_scrolling_body_leaves_the_column_alone` |
| the offset MOVES the islands | capture pair `/tmp/gpui_before.png` vs `/tmp/gpui_scrolled.png` (`TFG_SCROLL=20`), cross-correlated: best shift **25 physical px = exactly 20 window-logical px**, error 21678 against 347287 at zero shift |
| nothing else moved | `cargo test --lib` 474 pass, same 4 pre-existing failures; both binaries and `proto/gpui-island` build |

**The gesture itself is NOT verified on the real binary, and cannot be here.**
GPUI does not present on Xvfb, and `uimouse.py`'s uinput pointer moves the real
cursor but its buttons and wheel never reach the window
(`scripts/ui-shoot/README.md`, "Where the Wayland path stopped" — measured, not
assumed). So the wheel is covered by the harness test that dispatches the real
event, and the paint half is covered by a captured `TFG_SCROLL`, which is why
that seed exists. Saying "verified by capture" about the gesture would be
false.

**A process bug this turned up, and it invalidated a test.** `SHEET` was read
with `std::env::var` inside `render` and two tests set it to `1` without ever
clearing it, so every test that ran after them in the same process rendered the
roster sheet instead of the console. It masked a second bug:
`opening_the_reminder_opens_the_step_modal` asserted on `demo.reveal`, a field
`render` consumes with `reveal.take()` on the very next frame — it only passed
because the `SHEET` global made `render` return before the column ran. The
sheet is now a field seeded once in `new()` (where every other `TFG_*` seed
lives), the dead inner `else if SHEET` branch in the map child is gone, and the
test asserts the outcome the comment claims: the click opens the island AND the
column scrolls it onto the band. Rule: a process-global read in `render` is a
race with every other test in the binary, and a test that asserts on a field
the frame is designed to consume is asserting on nothing.

### The Planning column, the console's own (2026-10-13)

The owner's three questions. The third was a bug; the first and second were a
diagnosis that contradicted notes this log had already recorded, so both are
written down as measurements.

**1. The data was right, but the read was incomplete.** Game 45 ("Game 15",
planning) genuinely has **0 participants** — `GET /games/45/participants` answers
`{"participants":[]}`, and `/units` and `/scenarios` are empty too. Nothing was
misparsed. What was missing is that `fetch_session` never called
`game_detail`, `game_readiness` or `game_hierarchy`, and the kernel's own
`zone_islands` doc justified three missing islands with *"no read route exists
for either here"*. All three routes exist, are implemented in `master.rs`, and
answer — `/games/45` returns name, mode, pace, prose and `time_factor: 1`;
`/readiness` 404s until preparation; `/hierarchy` returns `[]`. So the notes
were wrong and the comment now says which routes answered what.

**2. The comparison.** The console's Planning column is seven islands
(`main.rs:11625–11665`); the shell had three.

| console | h | shell had | what was wrong |
| --- | --- | --- | --- |
| Operator | 172 | OPERATOR | close; trailing differs (below) |
| Essentials | 380 | — | absent |
| Control | 182 | — | absent |
| Fleet | 320 | FLEET | **wrong surface**: the whole 127-hull REGISTER, which is the console's *picker modal* content. The console's Fleet is the DRAFT, plus a door |
| Players | 280 | **CREW** | **renamed** — and CREW is a real, different console island (`z.crew`, the arrival list), so the old name named something else. This broke the shell's own rule in `IslandKey::title()`'s doc |
| Review | 380 | — | absent |
| Groups | 380 | — | omitted, named: the console's own local-scratch authoring, no read route |

Planning now carries all six the data supports, at the console's heights and in
the console's order: Operator(172), Essentials(380), Control(182), Fleet(320),
Players(280), Review(380). Stack 1726 against a 758pt band — the planning
column cannot fit any window this console runs in, which is exactly why the
scroll fix mattered.

Trailing notes follow the bodies: Fleet now reads `N PIECES` (the draft) and
not `127 HULLS`, Players reads `N SEATED`, Crew reads `N IN ROOM`, Control
reads the multiplier, Review reads the server's outstanding count. The three
new islands read from `GameFacts` and `ReadinessView` on `SessionView`, which is
`game_detail` plus `game_readiness` flattened to what the column draws — and
`room_key` comes along free on the same payload, so Review's key needs no
second read.

**One divergence found and deliberately NOT ported.** In the console, an
island's close cross and its title-band drag are both vestigial: `open` and
`pos` are per-frame locals (`side_zone`), discarded on every redraw, so neither
persists. The shell's closable islands are kept and the divergence is recorded
here rather than copied.

**Writes.** Control's Apply posts `set_time_factor` and Review's
"Enter execution →" posts `transition_game` — both through the existing
`ToBinary` + re-read plumbing, so the readout on the next frame is Minos's and
never the draft's. Review's advance is ungated by role and by phase, matching
the console's reasoning: what may advance is the server's policy call, and a
client that hid the button would be one more copy of a rule it does not own.

### Verification

| claim | how it was checked |
| --- | --- |
| the planning column is the console's | `every_state_has_the_column_it_should` — six keys in order; `the_column_follows_the_state_of_the_exercise` — no-seat and player-station planning |
| the stack arithmetic still holds | `the_stack_height_counts_a_gap_per_island` (1726), `the_column_scrolls_only_by_what_it_overflows` (1186 of overflow in a 540pt band) |
| the wheel still chains | `a_wheel_over_the_column_moves_the_column`, `a_wheel_over_a_body_with_room_leaves_the_column_alone` |
| the column scrolls and the bodies are reachable | captures `/tmp/gpui_plan_top.png` and `/tmp/gpui_plan_sc.png` (`TFG_SCROLL=900`): the first island on band moves from OPERATOR (band edge physical y 74) to PLAYERS (y 264) |
| the new islands draw real server data | same capture: `Game 15 · scenario · planning · standard pace`, `window incomplete` (amber — both ends or neither), `nothing written yet`, `×1× · Pace 1× — read from Minos`, Apply disabled while the draft is clean |
| no regressions | `cargo test --lib` 474 pass, same 4 pre-existing failures; both binaries and `proto/gpui-island` build |

**Two things in the wheel test, both failures of the test rather than the code,
recorded because both cost time.** One `simulate_event` is dispatched through
every listener the rendered frame accumulated, so a `simulate_mouse_move` that
notifies paints again and both paints' listeners survive into the dispatch —
one notch therefore moved the column by 80, not 40. The test now pins direction
and the ceiling, not the increment. And aiming a wheel at an island requires
its SCREEN position, which is `origin - scroll`: computed from the unscrolled
origin the gesture lands below the window and moves nothing, which is how the
column half passed while the body half was never reached.

**Still open.** The two pickers — the Fleet register as a modal with
`ForceDraft` staging, and the Player picker — are the next slice. `tfg::force`
and `tfg::roster` are toolkit-free and already carry the console's own
`plan(draft, live)` write generation, so the staging is portable rather than
invented; the reason it is not here is that it is a real slice on its own.
Groups is still omitted. The trailing-glyph notdef is still parked at the
owner's request.

### The Fleet picker, on the console's own staging (2026-10-13)

The Fleet island's door now opens something that works. It is the console's
`ForceDraft` and `force::plan`, ported rather than reinvented — `src/force.rs`
is toolkit-free, so the draft, its rules and its write generation are the same
code the egui console runs.

**Every edit posts at once.** The console stages and syncs at a stage advance;
this shell has no advance, and posting at once is its pattern for every other
authoring surface (the book, relate, readiness all do). The consequence that
makes it safe: the draft can be re-seeded from any session read, because it
never holds anything the server has not been told — so a telegram answer's
re-read cannot discard staged work.

**The plan is one message, not one per edit.** `ToBinary::SyncForce` carries
every write `plan` owed, in the order the server accepts them, and the first
refusal stops the plan. Posting a hull and its commander as two messages would
let a failure between them leave the exercise holding a piece nobody commands —
the exact state `ForceGap` exists to prevent.

**One rule, one place.** `stage_assign` and `stage_handover` TAKE a commander
rather than checking it. Whether an account may hold a piece is the picker's
question (it offers the room's seats and nothing else); whether a hull may be in
the exercise without one is `force::plan`'s. A first pass put the seat check in
both, and the test caught it asserting a sentence the kernel never wrote — two
copies of one rule, where the copy that drifts is the one that writes.

**What the picker does NOT do.** No placement. The console places a hull by
dropping it on the map; this shell has no map drop, so neither the draft nor its
`LiveHull` mirror carries a start and `plan` emits no `Place` or `Lift`. A
`Place` or `Lift` reaching the binary arm is reported loudly rather than
swallowed, because it would mean the draft grew a coordinate nobody chose.
Placement is still READ — `PieceView.placed` from `GET /games/{id}/placements` —
and still shown on both the island and the picker.

**`WhichPanel` gained `None`.** `open_only` takes the whole enum, so a modal
close is "open nothing" rather than four flags flipped by hand. The picker's ✕
uses it, and it is the shape every future door should use.

### Verification

| claim | how it was checked |
| --- | --- |
| a pick posts one Assign carrying the commander | `a_picked_hull_with_no_commander_is_refused_and_named` (asserts on the `ToBinary::SyncForce` payload) |
| the kernel's rule stops the whole plan and names the hull | `an_uncommanded_piece_stops_the_whole_plan_and_is_named` — through `sync_force`, with nothing sent |
| a remove is one Remove, and a hull never drafted is not a write | `a_removed_piece_is_one_remove` |
| the write endpoints work for this account and game | **live**: `POST /games/9/units {id_unit:55, id_commander:2}` → **201**; `DELETE /games/9/units/55` → **200**; game 9 back to `[8, 12]`, exactly as found |
| the picker draws a real seated room | capture `/tmp/gpui_picker9b.png` (`TFG_GAME=9 TFG_FLEET=1`): `IN THE EXERCISE — 2` with commanders, placed state, the room's three seats as handover buttons and remove; the register with `add`, and `assigned` (disabled) on a hull already in the exercise |
| the gate is honest with nobody seated | capture `/tmp/gpui_picker.png` (game 45): "Nobody is seated, and Minos gives every piece a commander — seat the room on the Players island first." |
| no regressions | `cargo test --lib` **477 pass**, same 4 pre-existing failures; both binaries and `proto/gpui-island` build |

**Game 9 is the planning room this was measured on** — 3 non-judge seats (Super
User, Budi Santoso, Markus Wijaya) and two pieces already assigned, which is the
only dev game in a state where a pick can be made at all. Game 45 is the bare
planning room: 0 seats, 0 units, no book.

**Still open.** The Player picker — seating, unseating and role changes through
`tfg::roster`'s `RosterDraft` and its `plan` — is the next slice; the console's
`z.players` door opens it and the Players island here does not yet have one.
Groups is still omitted. The trailing-glyph notdef is still parked at the
owner's request. And island BODY scrolling remains unverified by gesture: GPUI's
built-in scroller does not fire for a body inside the clipped column in this
harness, so the column's scroll is proven and a body's is not.

---

## The Player picker, on `tfg::roster`'s own draft

The Players island's door now opens the directory, and seating, unseating and
role changes all go through the console's own `tfg::roster` — the same module
the fleet picker runs on, ported whole rather than reimplemented.

**The door is the console's.** `z.players` has one because seating is one
decision about a person and a 280-point column cannot hold it. The directory
needs no extra read: `fetch_session` carries it, so opening the panel draws the
real thing rather than an empty form with no read attempted.

**One plan, one post.** `stage_seat`/`stage_unseat` mutate the draft, then
`sync_roster` calls `roster::plan(&draft, &live)` and posts the whole result as
one `ToBinary::SyncRoster`. Same shape and same reason as `SyncForce`: a plan is
one decision, and posting its writes one message at a time would let a failure
between them leave a half-seated room. Safe because the draft is a mirror of
what Minos was last told, so a session read can re-seed it — and both drafts are
now re-seeded by the ONE call `reseed_drafts`, which replaced `reseed_force`
because two reseeds would be two chances to do one and forget the other.

**`PersonView` gained `role_id`, and this was not cosmetic.** `roster::plan`
compares staged seats against the server's BY ID, so a seat re-picked as the
same role has to converge to nothing. Carrying only the role's name made both
sides incomparable — `live_seats()` had to fill in `0` — and every restage
looked like a change, i.e. a guaranteed 409 on any re-pick. The test
`restaging_a_seated_account_as_the_same_role_writes_nothing` is what caught it:
it failed first, and the fix was in the model, not the test.

**Two records, both named.** `directory` and `roles` are on `SessionView` and
read at hold time. The roles are the game's own `/games/{id}/roles`, never a
hardcoded list — a deployment that relabels a seat keeps working. The directory
is a question about ACCOUNTS rather than the exercise, which is why the console
reads it on the way into the picker and why this reads it with the session.

**No invented call sign, and it says so.** Minos requires one on a seat and the
console's form asks the operator. Here the account's own name is used,
upper-cased and cut to eight, and the picker names what it used rather than
presenting the choice — a seat form that silently invented a call sign would be
a call sign nobody chose.

### A GPUI trap worth its own entry

`overflow_y_scroll` is on `StatefulInteractiveElement`, not `InteractiveElement`.
A plain `Div` has no such method. It only exists once the element is stateful —
so `.id(...)` MUST come first in the chain:

```rust
// works
div().id("x").flex_1().overflow_y_scroll()
// does not compile
div().flex_1().overflow_y_scroll()
```

The symptom is misleading: the error reads "no method named
`overflow_y_scroll` found for struct `gpui::Div`", which points at the wrong
thing entirely.

**And both pickers were clipping their lists with no scroller at all.** The
Fleet picker only got away with it because game 9's force is two hulls; the
Player picker's eight accounts and five roles overflowed a 420-point panel on
first render. Neither modal had a scroller — they ended in `.overflow_hidden()`,
so content was simply unreachable. Both are now a fixed header over a scrolling
list (`#player-picker-list`, `#fleet-picker-list`).

**Directory rows are two lines, not one.** One row with the name and five role
buttons after it loses whichever end it ran out of: the name clipped, or the
roles slid under the panel's edge. Stacked — name on one line, the role buttons
wrapping below — both survive a name of any length and a role list of any size.
The row's detail joins only the parts the mirror actually carries; a blank rank
joined with a separator reads as a row that lost its middle.

### Verification

| claim | how it was checked |
| --- | --- |
| seating an unseated account is one Seat carrying a call sign | `seating_a_directory_account_is_one_seat` (asserts on the `ToBinary::SyncRoster` payload) |
| a restaged seat as the same role writes nothing | `restaging_a_seated_account_as_the_same_role_writes_nothing` — the test that caught the missing `role_id` |
| an unseat is one Unseat | `an_unseat_is_one_write` |
| the write endpoints work for this account and game | **live**: `POST /games/9/participants {id_user:5, id_game_role:19, call_sign:"TFGADMIN"}` → **201**; `DELETE /games/9/participants/5` → **200**; game 9 back to `[2, 6, 13]`, exactly as found |
| the picker draws a real seated room and a real directory | capture `/tmp/gpui_player9c.png` (`TFG_GAME=9 TFG_PLAYER=1`): `IN THE ROOM — 3` with each seat's role and steers/directs, `THE DIRECTORY` with rank · unit · appointment and five role buttons per account, `5 of 5 accounts` |
| the door is on the island | capture `/tmp/gpui_player_door.png` (`TFG_GAME=9 TFG_SCROLL=760`): Players reads `Game 9 · 3 seat(s) · 0 ready · you: Commando`, then `Player picker (3 seated)` |
| the pickers scroll | captures above: the directory's last row (`System`) sits past the panel's lower edge with the header fixed |
| no regressions | `cargo test --lib` **480 pass**, same 4 pre-existing failures; both binaries and `proto/gpui-island` build |

**The readiness dash in the Players island is honest.** It shows `—` for all
three seats because `is_ready` is false for all three; nothing was broken, the
column is the console's own "ready" badge reading a room that has not declared.

**Still open.** Groups remains omitted — the console's own is local-scratch
authoring with no read route, so naming the surface would name something empty.
Island BODY scrolling is still unverified by gesture for the same reason as
before (GPUI's built-in scroller does not fire inside the clipped column in this
harness); what is now proven is that the MODAL bodies are scrollers at all and
that their content is reachable. The trailing-glyph notdef is still parked at
the owner's request.

## The three columns nobody had looked at (2026-10-13)

The Planning column was built and captured island by island. The other three
states — the ones `zone_islands` composes for `Preparation`, `Execution` and
`Closure` — were pinned by kernel tests on their *membership* and had never
been driven or seen. Every capture since the recomposition was a planning game
(9, 45); the execution captures in this log (`gpui_comms.png`, `gpui_step.png`,
the helm shot) all predate the band and the dock.

That mattered more than a missing screenshot, because execution is the state the
console spends its life in. It is now seen, and the columns hold.

### What each state's column is, and where it was seen

| state | column | seen on |
| --- | --- | --- |
| Planning | Operator, Essentials, Control, Fleet, Players, Review | game 9 / 45 (earlier entries) |
| Execution, player station | Operator, Step | `/tmp/gpui_exec44.png` — game 44 |
| Execution, judge | Operator, Step, Command, Crew | `/tmp/gpui_exec_gm.png`, scrolled `/tmp/gpui_exec_gm_sc.png` — game 19 |
| Preparation, judge | Operator, Command, Crew | `/tmp/gpui_prep38.png` — game 38 |
| Preparation, player station | Operator, Command, Crew | `/tmp/gpui_prep41.png` — game 41 |
| Closure | Operator, Crew | `/tmp/gpui_closure11.png` — game 11 |

`player` is `own_seat().is_some_and(|p| !p.judge)`, the console's
`player_station()` verbatim. Game 44 seats this account as `Commando` (call sign
`SUEX`) and game 19 as `Penyelenggara`, which is what put the two halves of the
execution column in reach without inventing a credential.

**A player station's execution column is two islands and fits.** Operator 172
plus Step 380 plus two gaps is 556 against a 758-point band, so it never
scrolls at all. The judge's is 172+380+420+240 plus four 2-point gaps = 1220 over a 758-point
band, so its ceiling is 462. `TFG_SCROLL=474` asked for more than the ceiling
and clamped to it, which lands Crew — whose origin is 978 — 516 points down the
band with all 240 of its height inside the 758. The ceiling decided the seed;
a number above it would have been clamped to the same photograph.

### What the islands read, and that it is the server's own answer

**Crew's count is arrivals, not seats, and it is right.** Game 19 seats three
people and badges two: Budi Santoso and Markus Wijaya both have `joined_at`, the
judge seat does not, so the island reads `2 IN ROOM` and gives the third row
`judge` with no arrival badge. Game 11 also seats two with neither arrived and
reads `0 IN ROOM`. The console's line — "seated is a role; in room is arrival" —
is doing real work in both captures rather than restating the seat count.

**Command's exemption sentence is the judge's answer, and it is not a bug.**
Game 19 and game 38 both show `Super User · Penyelenggara — judge side is exempt
from readiness`, then `No hull assigned to this seat.` Game 41 — same account as
`Commando`, ready — shows the other branch: `ready to start` with an armed
`withdraw`, `1 HULL`, and `KRI Abdul Halim Perdanakusuma (KRI-355) 255° 0.1 kn`
with a `stale` badge. Four states of `readiness_ui`, all four drawn from what the
server returned.

**The step verbs stay off, and say why.** In game 19 the judge sees
`no step asks you`; in game 44 the player sees `Asking / Take station east of
the buoy / First move · 0 of 1 ready` with `DECLARE READY` armed. Play, Skip, End
and Rewind are disabled in both, with the aside naming the reason — runs are
tracked by the Game Master and this shell tracks none. A disabled verb with its
reason attached is not a stub; it is the honest reading of a capability the
shell does not have.

### One branch with no live route

**`zone_islands(NoSession, _)` cannot be reached by any capture.** It composes
Operator and Fleet, and a kernel test pins that. But this shell's sign-out takes
the whole window to the login card — `TFG_SIGNED_OUT=1` is the proof,
`/tmp/gpui_nosession.png` shows `ARCONS` with a Sign in form and no column at
all — so a signed-in account holding no game is a state the binary cannot enter:
`fetch_session` pins a game or takes the first of `games_list()`, and no dev
account has an empty list. The console reaches that column because its landing
surface *is* the register; here the column is replaced by a login card.

That is a named divergence, not a defect, and it is recorded rather than
papered over by a seed. Nothing in the shell can compose it, so nothing can
verify it beyond the kernel test.

### Verification

| claim | how it was checked |
| --- | --- |
| a player station's execution column is Operator + Step and fits | capture `/tmp/gpui_exec44.png`: two islands, band reads `NEXT · First move — Take station east of the buoy · 0 of 1 ready` |
| a judge's execution column adds Command and Crew | capture `/tmp/gpui_exec_gm.png` (game 19): COMMAND with the exemption line and the helm, CREW below the fold |
| the judge column's scroll reaches the last island | capture `/tmp/gpui_exec_gm_sc.png` (`TFG_SCROLL=474`, clamped to the 462-point ceiling): CREW `2 IN ROOM` wholly inside the band |
| Crew counts arrivals, not seats | game 19: three seats, `joined_at` on two → `2 IN ROOM`; game 11: two seats, neither arrived → `0 IN ROOM` |
| Command draws all four readiness branches | game 19 / 38 judge-exempt; game 41 `ready to start` + `withdraw` + `1 HULL` + `stale` |
| the step verbs are off for a stated reason | both execution captures: Play / Skip / End / Rewind disabled with the runs aside |
| preparation and closure columns draw | `/tmp/gpui_prep38.png` (judge, `0 IN ROOM`), `/tmp/gpui_prep41.png` (player, hull), `/tmp/gpui_closure11.png` (Crew only) |
| the no-session column has no live route | `/tmp/gpui_nosession.png` (`TFG_SIGNED_OUT=1`): login card, no column |
| no source changed | this entry added no code; `cargo test --lib` unchanged at **480 pass** with the same 4 pre-existing failures |

**Still open, unchanged.** Groups remains omitted — no read route. Island BODY
scrolling is still unverified by gesture, and these captures do not touch it: no
column here needed a body scroller, because every island fits its band once the
column itself scrolls. The trailing-glyph notdef is still parked.

## The band's three doors, and a capture script that was lying politely (2026-10-13)

The band opens three modals — `+ New session`, `comms` and `Settings` — and none
had a capture on the current band. Settings had no capture anywhere in this log,
comms had one that predates the band rewrite, and `+ New session` opens the
Composer, which had none.

| door | capture | what it reads |
| --- | --- | --- |
| `Settings` | `/tmp/gpui_door_settings.png` | VIEW `Watch only`; LAYOUT `Side zone` checked, `Docked left`, `Width − 360 + reset`; MOTION `Reduce motion` |
| `comms` | `/tmp/gpui_door_comms.png` | `Game 8 · 0 message(s)`, `No messages — and none can arrive until the game is executing.`, `New telegram` field, `SEND`, and the aside `Composing is a write, and the server takes messages only in execution.` |
| `+ New session` | `/tmp/gpui_door_compose.png` | `Name *` with `e.g. Latihan Armada 2026`, `Mode` (maneuver / scenario / static), Description, Purpose, Target, Area, Map tag, `Cancel` and `Create session →` greyed with `name required` |

**All three are real surfaces, and two of them refuse honestly.** Comms' SEND is
disabled because game 8 is planning and messages are an execution write; the
Composer's Create is disabled because the name is empty, and it says `name
required` rather than sitting inert. A disabled control with its reason attached
is the reading of a capability the shell does not have, which is not the same as
a stub.

### The capture script was writing plausible wrong pictures

The first three attempts at these captures returned a fullscreen video — while
`/tmp/gpui_run.log` showed the app drawing perfectly (`map frame 24: 237ms`,
`fit island=review`). Nothing was wrong with the shell.

**`grim -o eDP-1` screencopies the composited output, which is the ACTIVE
workspace only.** The app lands on a different workspace each launch
(`special:scratchpad`, then `5`, then `1`), and a window on another workspace is
not mis-cropped — it is absent. Whatever is on top at its coordinates is written
to the PNG instead, and it looks entirely plausible.

**It cannot be worked around here.** `hyprctl dispatch` is broken in this build:
it wraps the argument in Lua (`hl.dispatch(focuswindow class:tfg-gpui)`) and
cannot parse a multi-word one. This is over the CLI *and* the raw IPC socket —
`printf 'j/dispatch workspace special:scratchpad' | socat - UNIX-CONNECT:$SOCK`
fails identically. The recipe's note that dispatch "fails in this shell with a
Lua parenthesisation error" was right, and now it is known to be unfixable from
this side rather than merely inconvenient. So the workspace cannot be switched
to, and the capture cannot be forced.

**So the script now REFUSES instead.** Rule 5, and it is the same shape as the
four the script already carries: read the window's workspace alongside its rect,
compare it to the active workspace, and exit 5 with a named error naming both
workspaces and the reason. A named error is recoverable; a wrong PNG is a
debugging pass spent proving the shell is fine, which is exactly what this
morning's session was.

```
window rect (logical): 768 26 768 838  on workspace '5'
captured /tmp/gpui_door_comms.png  (466714 bytes)
```

The rect line now names the workspace too, so a capture that succeeded says
where it came from and one that could not have says why.

### Verification

| claim | how it was checked |
| --- | --- |
| the Settings door opens a real modal | `/tmp/gpui_door_settings.png`: VIEW / LAYOUT / MOTION with `Side zone` checked and `Docked left` |
| the comms door opens a real modal and refuses honestly | `/tmp/gpui_door_comms.png`: `0 message(s)`, the empty state, SEND present, the execution-only aside |
| `+ New session` opens the Composer | `/tmp/gpui_door_compose.png`: required Name, three modes, five free-text terms, `Create session →` greyed with `name required` |
| the app was drawing while the wrong pixels were captured | `/tmp/gpui_run.log`: `map frame 24: 237ms (pump 4)`, `fit island=review content=350 viewport=350` |
| the app's workspace varies per launch | three runs reported `special:scratchpad`, `5`, `1` |
| workspace switching is unavailable | `hyprctl dispatch` fails over CLI and over `socat` to the raw socket, identically |
| a wrong capture is now refused, not written | script rule 5, exit 5, naming both workspaces |

**Still open.** `WhichPanel::Book` has no door — reachable only by `TFG_BOOK=1`
and by two tests. The console's own band has no book door either (it was ported
verbatim), so this is a question about where the console opens it, not a missing
button to invent. Groups remains omitted, island BODY scrolling remains
unverified by gesture, and the trailing-glyph notdef remains parked.

## The scenario composer's door, on the island that owns the plan (2026-10-13)

`WhichPanel::Book` — the whole API-verified scenario and step surface — had no
way in. It was written by `TFG_BOOK=1`, by `open_only`, and by two tests, and
by nothing an operator could press. The handoff had recorded that as a question:
is the console's band missing a door too, or does the console open it from a
surface not ported?

**It opens from Essentials.** The console's own placement, verbatim in
`main.rs::setup_session_ui`, with its comment intact:

> The scenario book's entry point, on the island that already owns the held
> session's own facts.

And its gate, also verbatim: **absent outside planning rather than greyed out**,
because authoring is a planning-time act and the server refuses it after, so a
control that would 403 is a lie about what this session can do.

So the missing door was never a band door. It is the third door of the same
shape as the two already built — Fleet's island doors the Fleet picker, Players'
island doors the Player picker, and now Essentials doors the composer:

| island | door | what it opens |
| --- | --- | --- |
| FLEET | `Pick hulls` | `WhichPanel::Fleet` |
| PLAYERS | `Player picker (n seated)` | `WhichPanel::Player` |
| ESSENTIALS | `Scenario composer` | `WhichPanel::Book` |

The island is the summary; the modal is where the decision gets room. That is
the console's whole arrangement for a panel too large for a 320-point column,
and three of them now follow it rather than each inventing a shape.

### What was added

One door on `essentials_body`, gated on the state, carrying the console's own
readout beside it: `N scenario(s), M step(s).` when the book is non-empty. The
body's `cx` parameter was `_cx` until now — Essentials had no handler, which is
itself the evidence that the island was a pure read.

The body-level gate is the console's own belt-and-braces, not the load-bearing
one: `every_state_has_the_column_it_should` already pins Essentials as
planning-only, and the kernel test fails before the door can be reached from
another state. It is kept because it states the reason where a reader looks —
inside the body that would otherwise draw the button.

**No new test.** The two claims are already pinned, and adding a third would
restate them: the island is planning-only
(`view::every_state_has_the_column_it_should`), and opening the Book is
exclusive (`shell_gpui::only_one_modal_opens_at_a_time`, which already asserts
`(modal_open, book_open, composer_open) == (true, false, false)` after a
different door closes it). A test that drives a click on an element id cannot be
written in this harness — the existing tests call the handler directly — so it
would pin the gate twice and the behaviour not at all.

### Verification

| claim | how it was checked |
| --- | --- |
| the door is on Essentials, in planning, after THE PLAN | `/tmp/gpui_book_door.png`: `Scenario composer` under `THE PLAN / nothing written yet`, beside the island's `PLANNING` tag |
| the door is absent outside planning | capture on game 19 (execution) showed Command and Crew and no Essentials island at all — the kernel's column decides, not the body |
| no scenario count line when the book is empty | same capture: game 9's book is empty and the line is not drawn |
| the console really places the door on Essentials | `main.rs::setup_session_ui`, `open_composer(None)` under the comment quoted above |
| the console has no band door for the book | `enum WhichModal` is Settings, Composer, Fleet, Player, Session — the band's three doors are Session, Settings and `mail_ui` |
| the count line's subject exists on some game | `GET /games/44/scenarios` → 1 scenario, 1 step (`First move` / `Take station east of the buoy`) |
| the capture refused rather than writing a wrong PNG | rule 5 fired twice in a row, naming workspaces `1` and `5`, before attempt 3 landed on the active one |

**The one line not seen.** `N scenario(s), M step(s).` has no subject on this
account: all seven planning games have an empty book, and the one game that
carries scenarios (44) is in execution, where the island is not in the column.
Making a subject would mean writing a scenario to a live dev game, which is a
mutating GM verb and needs the owner's word — the same rule that kept the shell
from composing a telegram. The line is a format string over `session.book` and
`steps.len()`, and the door around it is verified.

## What the console lists and this shell does not (2026-10-13)

Asked whether the migration is finished, the answer was checked rather than
assumed, and the console's own column specs settled it: `main.rs::side_zone`
pushes **15** island bodies across six states, and `zone_islands` names **9**.
The recomposition is done and verified; the port is not.

This is the table. It is the one place to look, and the kernel's arms now point
here rather than each repeating part of it.

### The console's columns, verbatim from `side_zone`

| state | the console's islands, in order |
| --- | --- |
| NoSession | `z.start` |
| Planning | `z.essentials`, `z.control`, `z.fleet`, `z.groups`, `z.players`, `z.review` |
| Preparation | `z.ready`, `z.essentials` |
| Execution (organizer) | `z.clock`, `z.orders`, `z.steps`, `z.crew`, `z.log` |
| Execution (player) | `z.mystep` |
| Closure | `z.assessment`, `z.log` |

`z.user` ("Operator", 172) is pushed before the match, so it leads every state —
which is why `IslandKey::Operator` is first in every one of mine, at the
console's own 172.

### What was missing, and whether it was already named

**Log and Assessment are PORTED now — see "Log and Assessment, and a cross
that was lying".** The table below is the state as it was found; `Groups` is the
only row still standing.

| console island | height | states | named? |
| --- | --- | --- | --- |
| `z.groups` "Groups" | 380 | Planning | **yes** — `zone_islands`' Planning arm, "no read route at all" |
| `z.log` "Log" | 260 | Execution, Closure | **was** Closure yes, Execution no — the Execution arm named the Exercise merge and never mentioned Log. Now ported, the feed half. |
| `z.assessment` "Assessment" | 420 | Closure | **yes** — `zone_islands`' Closure arm. Now ported, four of five tabs. |
| `z.clock` "Exercise" | 200 | Execution | **yes** — merged into Step, `zone_islands`' Execution arm |

So the port was more honest than first believed: three of the four gaps were
named where they were made. **The gap was that none of them reached the
handoff**, so a reader of the log had no way to know a fifth of the console's
islands were absent. `z.log` in Execution was the one genuinely unnamed case.

### Two columns carry an island the console's does not

Not every difference is an omission, and this table would be a lie if it only
listed what is missing.

- **Preparation** — mine is `Command, Crew`; the console's is `Readiness,
  Essentials`. Crew is a *substitution* for the terms surface the shell has no
  island for, not a port of anything. **The kernel's comment said "Players
  rides along" while the code pushed `Crew`** — a stale description of a column
  that was never built, fixed here, with the divergence now stated in the arm.
- **Closure** — mine is `Crew`; the console's is `Assessment, Log`. Crew is
  kept because a debrief is about who was in the room, and that much was named.

### What is blocked, and what is merely not done

The distinction matters because it decides whether this is a decision or a task:

- **Groups — blocked.** `zone_groups_body` is 139 lines of local scratch
  authoring with no read route at all. Nothing to port to.
- **Exercise clock — blocked.** Needs the Minos clock channel, and its
  `End session →` is a transition write that needs the owner's word.
- **Log and Assessment — NOT blocked.** `zone_log_body` is six lines around
  `log_island`; `zone_assessment_body` is three lines delegating to
  `assessment_body`. Both were simply never ported, and both are now the
  obvious next slice.

### Correction to the last entry

The entry before this one claimed Closure's Assessment and Log were "never
named". That was wrong — `zone_islands`' Closure arm names both, in the
console's own words. What was true, and is what this section fixes, is that
none of the four gaps reached the handoff.

### Verification

| claim | how it was checked |
| --- | --- |
| the console has 15 zone bodies | the `match id` in `zone_side_bodies`, `z.user` through `z.assessment` |
| `z.user` leads every state | `side_zone` pushes it at line 11601, before the per-state match |
| Log belongs to Execution too | `side_zone`'s Execution arm pushes `z.log` (260) beside Steps and Crew |
| Assessment is Closure-only | `side_zone`'s Closure arm; the only `z.assessment` push |
| the console's Preparation has no Crew | its arm pushes `z.ready` and `z.essentials` and nothing else |
| Log and Assessment are small | `zone_log_body` 6 lines, `zone_assessment_body` 3 lines |
| Groups is genuinely unportable | `zone_groups_body` 139 lines, and no read route exists |

## Log and Assessment, and a cross that was lying (2026-10-14)

The next slice, closing Execution and Closure to parity. `z.log` and
`z.assessment` were the two islands the previous entry found missing, and the
reason Log was worth doing first is that it is not a read at all.

### LOG — a feed, not a read

The console's `z.log` is two surfaces sharing an island:

- **An event feed**, `event_feed: VecDeque<String>` capped at 30 by `feed()`
  (`main.rs:17029`), which also echoes to stderr.
- **The session's journals**, below a rule, with a replay slider that drives
  map ghosts.

The feed is carried and the journals are not, because the journals are files on
disk and the ghosts are a replay of them — the same class of gap as Groups, and
worth saying in the island rather than leaving a 260pt panel that reads as
unfinished.

**The important part: there is no events GET.** Every line in the console's own
feed is a line that client produced — `feed()` is called from the write paths,
never from a read. So the shell's `log_feed` is the same: pushed by
`set_readiness`, `set_time_factor`, `sync_force`, `sync_roster` and
`declare_step`, each with a line naming what was asked. Nothing seeds it on
launch, because a feed invented at startup would be a log of things that did
not happen here.

That makes the feed unverifiable by capture — the only way to get a line into
it is a real write, which needs the owner's word — so it is verified by test
instead, and the tests are written to catch the failure a capture would miss:
a feed that logs without sending, or sends without logging, both look fine on
screen.

### ASSESSMENT — one tab of five

`z.assessment` is five tabs: Summary, Timeline, Judgements, Reviews,
Transcript. **Only Summary is carried**, and the other four are named in the
island rather than drawn as empty tabs — a tab strip over one live tab is
scaffolding.

This is a smaller omission than it looks, and the reason is worth recording:
**Timeline, Judgements and Reviews all already have backend reads in this
crate** — `timeline_page` (paged, 50 at a time), `judgements_list`,
`reviews_list`. What is missing is that the shell's own `fetch_session` does not
call them. So the four tabs are absent because the shell does not fetch them,
not because Minos cannot serve them, and the island says so in those words.
Transcript is the genuinely unportable one: `self.transcript` is a local trace,
"the session left no local trace", and there is no route.

The Summary is a read-back of the game's own shape once it is over — session,
exercise state, seats, pieces, clock factor — which is all data the shell
already holds.

### Two defects found on the way, both fixed

Neither was introduced by this slice; both were pre-existing and one was made
worse by it.

**The close cross was drawn on every island and only four responded.**
`set_island_open` ignores Operator, Fleet, Essentials, Control and Review, but
the band built its cross unconditionally — so nine islands carried a control
that read as clickable and did nothing. That is worse than no control, because
the operator cannot tell "it did not work" from "it is not there". Fixed with
`island_closeable`, and the cross is now the last child only where it does
something.

This is a deliberate divergence from the console, not a port: `chrome::
island_owned` sets `*open = false` and the spec persists it, so the console
closes ANY island. The seven that stay open here are the ones whose absence
would leave the column unable to answer the question its state exists to
answer — close Essentials and planning cannot say what is being planned.

**The cross was painted over the trailing note.** The note is a plain child
pushed right by `justify_between`; the cross is absolute at
`island_close_center`. A capture showed `3 SEATED` with the cross over its last
two glyphs — a count nobody can read. The pad is the kernel's now
(`island_band_pad_right`), asymmetric on purpose, and the reason it went
unnoticed is instructive: **the defect is invisible when the note happens to be
short.** `LOG`'s note is `—`, so the one island that showed it cleanly was the
one island with nothing to collide with.

### Verification

| claim | how it was checked |
| --- | --- |
| Closure is the console's own column now | `/tmp/gpui_closure11b.png`: OPERATOR, ASSESSMENT, LOG — the Summary read-back (`Game 9`, `CLOSURE`, `SEATS: 2`, `PIECES: 1`, `CLOCK FACTOR: 1×`) and the feed's empty state |
| Execution judge carries the feed | `/tmp/gpui_log19.png`: the LOG island below CREW, empty, with its journals aside |
| a write appends one line and still sends | `a_write_appends_one_line_to_the_log`: one line, it says "readiness", and `ToBinary::SetReadiness` reached the sink |
| the cap holds | `the_log_feed_keeps_thirty_lines`: 40 pushes, 30 lines, `line 10` first and `line 39` last |
| the cross is gone where it does nothing | `/tmp/gpui_nocross.png`: OPERATOR, ESSENTIALS and CONTROL bands carry their note and no cross |
| the cross is still there where it works | `/tmp/players_band2.png`: `3 SEATED` reads whole, cross clear to its right |
| the pad clears the button | `the_bands_right_pad_clears_the_close_button`, kernel |
| the three assessment reads exist | `timeline_page` (1409), `reviews_list` (1470), `judgements_list` (1676) in `backend/master.rs` |
| the console closes any island | `chrome::island_owned` sets `*open = false` (564) and the spec persists it (907) |

**Still open.** Groups (no read route), the Exercise clock (needs the Minos
channel; `End session →` is a transition write), island BODY scrolling by
gesture, and the trailing-glyph notdef. The three assessment tabs whose reads
exist but are unfetched are now the obvious next slice, and they need no owner's
word because they are reads.

## The Assessment island's other three tabs (2026-10-14)

The previous entry left Assessment carrying the Summary tab alone and named the
other four as absent. Three of them turned out to be **reads that already exist
in this crate** — `timeline_page` (paged, 50 at a time), `judgements_list`,
`reviews_list` — so the omission was that `fetch_session` did not call them, not
that Minos could not serve them. All three are now called and all three tabs are
carried.

### What is carried, and what is not

| tab | console's surface | carried |
| --- | --- | --- |
| Summary | the game's shape read back | yes, unchanged |
| **Timeline** | filters + `apply`/`reload` + event list + `load more` | **the list, read-only** |
| **Judgements** | subject picker + score + `record judgement` + the marks list | **the marks list, read-only** |
| **Reviews** | author + body + `file review` + `mine only` + the documents list | **the documents list, read-only** |
| Transcript | `self.transcript`, a local trace | no — no route, and its button is not drawn |

A tab strip over four live tabs is the console's own shape, so it stays. What is
dropped from each pane is the **write half**, and both writes are named at the
foot of the island rather than drawn disabled — a control that would be refused
is a lie about what this session can do. The console gates `record judgement` on
a judge-side seat and `file review` on authorship; neither gate can be honoured
here without the write, so neither control is offered.

### Three decisions worth recording

**The three reads are fetched only for a CLOSED game.** Assessment is a
Closure-only island, so asking Minos for a debrief of a game still being planned
is a read that state does not need — and on the evidence of the routes, one it
may refuse. Gating on the state rather than fetching always and hiding is the
honest version: the data is absent because it was not asked for, which is a
different sentence from "Minos has none", and the island already has a place to
say which.

**The events are `tfg::backend` types, not flattened copies.** `GameFacts` is
flattened because `GameDetail` nests a window struct this shell reads four
fields out of. `TimelineEvent`, `Judgement` and `Review` are flat already, and a
second struct holding the same fields is the "one answer served from two places"
this repo forbids. The shell's own comment at the flattening site says as much.

**`timeline_line` is ported verbatim, offset and all.** It reads Minos' `data`
blob by key, so its failures are a renamed field and a wrong shape — both of
which print something plausible rather than nothing. Every closure game on this
account returns `assumed_at` with a `+07:00` offset rather than a `Z`, and the
formatter's `strip_suffix('Z')` therefore keeps the whole tail, so a row reads
`11:58:42.414179+07:00 preparation → execution (by 2)`. That is the console's
own rendering and it was NOT tidied: a formatter the operator knows under one
output must not be renamed by a port, and "the console prints it ugly too" is
the answer to why this one is ugly.

### Verification

| claim | how it was checked |
| --- | --- |
| Timeline carries real events | `GET /games/4/timeline` returns `transition`, `clock` and `order` events; `/tmp/gpui_assess4.png` renders all three as lines |
| Judgements and Reviews are genuinely empty here | both answer `data: []` with `total_records: 0` for every closure game (4, 7, 8, 10, 11) |
| the tab strip shows which tab is on | `/tmp/tabs2.png`: `Timeline` in the bright ink on a tint, the other three dim |
| an empty pane says so rather than drawing nothing | `/tmp/tabs3.png`: `Judgements` picked, `No marks yet.`, then the writes aside |
| the formatter reads each event shape | `the_timeline_reads_each_event_shape`: transition, clock (with a zero `changed_by` printing no `(by 0)`), a clamped order, an unknown type, and a known type with a missing key |
| the writes are named, not drawn | both captures: `Recording judgements and filing reviews are writes — not carried here.` |

**Still open.** Groups (no read route), the Exercise clock (needs the Minos
channel; `End session →` is a transition write), island BODY scrolling by
gesture, and the trailing-glyph notdef. The two assessment WRITES now want the
owner's word, and the timeline's filters and `load more` are unbuilt — both are
interaction, not data.

## The parity proto compiles again (2026-10-14)

The Fleet picker, the Player picker and the Assessment island each added a
`crate::` path the proto does not shim — `force`, `roster` and `backend` — and
every one of those entries closed with "both binaries and `proto/gpui-island`
build". That claim was copy-pasted forward: the shell moved into
`src/shell_gpui.rs` at U8 step 3 with three shims, then grew three surfaces
that reached three more modules, and nobody re-ran the proto's build. It has
not compiled since. A parity proof that does not build is not a proof, and a
log that says it builds is the exact failure this document's own discipline
("a correction nobody can re-run is a claim, not evidence") exists to catch.

This is the prerequisite for the timeline's `load more` too: the page cursor
comes back as a `backend::TimelinePage`, and the shell cannot name it until
`crate::backend` resolves somewhere the proto can compile.

**The fix, and why each shim is the shape it is.** `force` and `roster` are the
console's own staging and toolkit-free, so they compile as `#[path]` to the
real files — the same one-implementation rule `view`/`book` already use.
`force` reaches `geo::coordinates` for `GeoPosition`, and the proto shims only
that leaf through a top-level `#[path]` re-exported as `geo::coordinates`,
never `geo/track.rs` (chrono + serde + a track registry the shell does not
draw). The re-export rather than an inline `mod geo { #[path] … }` is the trap
U8 step 3 already recorded — a `#[path]` inside an inline module resolves
against the inline module's own nonexistent directory — hit again and fixed
the documented way.

**`backend` is the one shim that is not a `#[path]`.** `master.rs` cannot
compile without reqwest, which is the whole thing the proto exists to stay
clear of, so the proto carries the three plain-data read types the shell names
(`TimelineEvent`, `Judgement`, `Review`) as shapes and nothing else. This is a
deliberate, bounded duplication: the shapes mirror `backend/master.rs`, and if
the shell ever reads a field the shim lacks the proto fails to compile — which
is the guard that keeps the two from drifting silently. It is a data shape, not
a rule, so it does not run afoul of the "one answer served from two places"
rule the repo enforces on logic. `serde_json` joins the proto's deps because a
timeline event's `data` payload is a `Value` the shell reads by key; it is pure
JSON with no transport, so it does not pull the HTTP stack back in.

### Verification

| claim | how it was checked |
| --- | --- |
| the proto builds | `cargo build` in `proto/gpui-island` → `Finished` (was 23 errors) |
| the shims are the real files, not copies | `force`/`roster`/`geo::coordinates` are `#[path]` to `src/` |
| the shipped crate is untouched | the only diffs are `proto/gpui-island/{src/main.rs,Cargo.toml,Cargo.lock}`; `cargo test --lib` 498 pass holds by inspection |

**One honest note, not fixed here.** The proto now compiles the whole 11k-line
shell but only calls `run_replay`, so it surfaces ~186 dead-code warnings for
the panels the replay path never draws. They are growth (the shell doubled
since the proto last built), not a regression from this change, and silencing
them with a crate-root `#[allow(dead_code)]` would hide real ones too. The
build succeeds; the noise is the cost of a parity artifact that compiles a file
most of which it does not exercise.

## The timeline's interaction half, and a binary that did not build (2026-10-14)

The previous entry's table was honest: Timeline carried "the list, read-only",
and the filters and `load more` were "unbuilt — interaction, not data". What
was not known is that the read half was broken too. `ToBinary::Timeline`
existed as a message — `send_timeline` posted it, with the cursor rules, the
busy latch and the console's own refusal sentences — but **no arm in
`tfg_gpui` handled it, no feed drained the answer, and no `apply` existed**.
`cargo test --lib` stayed green because it never builds the bins, and
`cargo check --bins` failed with `E0004: ToBinary::Timeline { .. } not
covered`. The same copy-pasted "both binaries build" the last entry caught in
the proto, one layer up. The Timeline tab drew the seed page and nothing the
operator asked for ever left the window.

### What was built, end to end

**The answer path.** `apply_timeline` is the console's `SetupDone::Timeline`
arm as a method: a reload REPLACES the list, `load more` EXTENDS it, either
way the cursor block is the page's and the latch clears, and a refusal lands
on the status line with the held events untouched. The status sentence is the
console's — `timeline: N event(s)`, plus `· more below` while a next page
exists.

**The wire.** `TimelineFeed` (the `(page, append)` pair, echoed back like the
console's own) is now a `run()` parameter drained in the tick loop, and the
binary answers it with `master.timeline_page` at the console's limit of 50 —
signed-out refused as a sentence, like every other arm. `run_replay` passes
`|| None`: a PNG replay has no Minos behind it.

**The tab.** The Timeline pane now draws the console's `timeline_ui` cut to
this shell's grammar: the source row (All plus the console's five streams —
changing it reloads from the first page, because a cursor never crosses a
filter change), personnel/unit and from/to fields (the hand-rolled grammar
the composer established), the console's BOTH verbs (`apply + reload` and
`reload` share the handler there too, so they share it here), the hint line,
the console's own empty sentence (`No events yet — reload to read the
exercise`, which replaced the shell's `the exercise left no timeline` — data
absent because it was not asked for is a different sentence from "Minos has
none"), the read's status line, and `load more →` only while a next page
exists.

**The cursor block moved to the session.** The seed fetch took page one and
kept `page.events`, dropping `next_cursor`/`has_more` — so on a closure
launch the tab showed fifty rows and could never offer more. The fix is where
the data lives rather than a second copy: `SessionView` carries
`timeline_cursor`/`timeline_has_more` beside `timeline` (one read, not two
answers), the shell's copies are deleted, and a session installed whole
carries its own block with it. `fetch_session` seeds all three from the page.

**The proto shim grew a fourth shape.** `TimelinePage` joins the three
plain-data shapes, and this is the guard working as designed: the shell named
a type the shim lacked, and the proto failed to compile until it was mirrored.

**Stale docs, corrected.** `zone_islands`' header still said personnel get
"the step and nothing else", that Assessment carries "the Summary tab and not
its Timeline, Judgements, Reviews", and that Execution's `z.log` "is ABSENT,
not merged" — all three contradicted the built column. The column test's doc
("shows only identity and the step") and the ASSESSMENT header ("the only tab
carried here") said the same old thing. They now describe the built surface.

### Verification

| claim | how it was checked |
| --- | --- |
| the bins build | `cargo check --bins` clean (was `E0004`) |
| the lib holds | `cargo test --lib`: 502 pass, same 4 pre-existing failures |
| reload replaces, `load more` extends | `timeline_reload_replaces_and_load_more_extends` (cursor block, latch, both status sentences) |
| a refusal keeps the held page | `timeline_failure_keeps_the_held_events` |
| a bad id refuses before any request | `send_timeline_refuses_a_bad_id_and_a_busy_tab` (personnel, unit, busy latch, nothing on the sink) |
| the cursor crosses only to `load more` | `send_timeline_carries_the_cursor_only_to_load_more` |
| the proto builds | `cargo build --manifest-path proto/gpui-island/Cargo.toml` (new shape mirrored) |
| the tab draws its filters and its events | `/tmp/gpui_timeline2.png` (game 4, `TFG_ASSESS_TAB=1`): source row, four fields, both pills, hint, four real event rows |
| the first capture lied | `/tmp/gpui_timeline.png` was taken against a stale `./target/debug/tfg_gpui` — the script runs the binary without building it. Rebuilt, re-shot. |

**Two things to carry forward.** The capture script never builds: every
`gpui-capture.sh` run needs a `cargo build --bin tfg_gpui` first, or the PNG
is evidence about the previous build. And the personnel field renders its
focused border on launch (`timeline_focus` defaults to 0) — the composer's
fields predate this slice and may share the habit; worth one look if the
column's focus grammar is ever reworked.

**Still open.** Groups (no read route), the Exercise clock (needs the Minos
channel), island BODY scrolling by gesture, and the trailing-glyph notdef. The
two assessment WRITES still want the owner's word. A click on `apply`,
`reload`, a source, or `load more` has kernel tests behind it but no driven
capture — gesture input does not reach the window on this machine, so the
binary arm's live round-trip is code-reviewed against the console's
`load_timeline`, not photographed.

## The debrief's reload verbs, and embarkations re-checked (2026-10-14)

With the Timeline interaction half done, the Assessment surface split cleanly:
everything left on it is either a WRITE (owner-gated) or a READ nobody had
asked for twice. The console's Judgements pane has exactly one read verb —
`reload`, always unfiltered — and its Reviews pane has two: `reload` and the
`mine only` narrow, which applies server-side on the reload like the inbox
filter. Both are now carried, and the survey that found them is worth keeping:
every other control on both panes is compose state for the two gated writes
(subject pickers, score field, body field, `file review` / `save revision` /
per-row `edit`), so drawing any of it would be a control that would be
refused — the lie the foot-of-island aside exists to avoid.

### What was built

**Two reloads, mirroring the timeline slice.** `ToBinary::ReloadJudgements`
and `ToBinary::ReloadReviews { author }`, each with its own feed
(`JudgementsFeed`, `ReviewsFeed`), its own latch and status line (the console
runs them as separate tickets), its own `apply_*` (REPLACE the held list —
both re-read whole, neither pages), and its own binary arm with the
signed-out sentence. The status sentences are the console's verbatim:
`judgements: N mark(s)`, `reviews: N document(s)`.

**The narrow resolves shell-side.** `author` rides the message as this seat
or `None`, because the binary holds the token but never the seating — and
`None` without a seat, the console's own flattening of a seatless
`auth_user_id` (`session.me` is 0 seatless, so the same match applies).
Flipping `mine only` reloads at once, the console's `.changed()` rule, which
is also why a narrowed reload REPLACES with the narrowed list rather than
filtering what is held.

**The panes.** Judgements draws `reload` over the marks list and its status.
Reviews draws `reload` beside a `mine only` checkbox (the shell's own U5
choice grammar, plain box like the console's `ui.checkbox`), then its status,
then the documents. The writes aside still closes both panes.

### Verification

| claim | how it was checked |
| --- | --- |
| the bins build | `cargo check --bins` clean |
| the lib holds | `cargo test --lib`: 504 pass, same 4 pre-existing failures |
| a reload replaces the marks | `judgements_reload_replaces_the_marks` (replace-not-extend, latch, busy gate, refusal keeps the held list) |
| the narrow resolves to the seat | `reviews_reload_narrows_to_mine_only` (Some(7) checked, None unchecked, None seatless, replace + status) |
| the proto builds | untouched by this slice — no new backend type named — `cargo build` clean |
| both panes draw | `/tmp/gpui_judge.png` (`TFG_ASSESS_TAB=2`): RELOAD, `No marks yet.`, writes aside. `/tmp/gpui_reviews.png` (`TFG_ASSESS_TAB=3`): RELOAD, unchecked `mine only`, `No documents yet.`, writes aside. Closure games hold no marks or documents on this account, so both panes are verbs over empty lists — the lists themselves were proven by the seed fetch, not invented here. |

### Embarked, re-checked with evidence

The "no embarkations exist to read" claim was a session old, so it was
re-probed rather than repeated: `GET /games/{id}/units/{unit}/embarkations`
for every unit of games 5, 6, 16, 17, 19, 42, 43, 44, 41 and 12 — 14 hosts —
and every answer is `{"units": []}`. The client's `parse_embarked_units`
reads the `units` key, so the shape parses to empty rather than erroring.
Embarked stays named, not ported — now on today's evidence, not inherited
lore.

**Still open.** Groups (no read route), the Exercise clock (its only control
is the owner-gated `End session →`; its readouts are the sim's tick, which
this shell has none of), island BODY scrolling by gesture, and the
trailing-glyph notdef. The two assessment WRITES still want the owner's word,
as do `zone_islands(NoSession,_)`'s fate and the player's helm click-proof.
The debrief's read half is now complete: every pane the console reads without
a write is drawn, and every verb on them asks Minos again.

## The debrief's writes, and the exercise's end (2026-10-14)

The owner said go — on the writes, on `End session`, and on the column
carrying them. A prior slice had surveyed both panes control by control, so
no new investigation was needed: the read verbs were taken, and what was left
was exactly the compose halves plus the clock island's one control.

### Record judgement

The console's compose in order: the seat banner, the subject pills, the
score, the verb. Subjects are pills over the held room minus the caller
(self-judging fails loudly server-side and the picker refuses it first),
sorted by name like the console's ComboBox options, with the stored pick kept
while it is still seated and the first option otherwise. The record verb uses
the console's `add_enabled` rule as a build rule: without a judge seat there
is no click target at all, not a pill that answers with a refusal — but the
send refuses anyway with the banner sentence, because the seam must not trust
the renderer. The guards run in the console's order with its sentences
(`pick a subject first`, `cannot judge yourself`, `write the score first`).
An empty room refuses with the subject sentence: the shell's subjects ride
the held session, so there is no numeric fallback to offer, and the cut is
stated where the pills would be (`No subjects seated.`).

The answer names the filed mark and re-reads through the reload — the
console's `JudgementSent` rule (`recorded #id · reloading judgements`), which
is why the write and the read hold separate latches on separate feeds.

### File and revise review

Filing picks any seat including this one (authorship is the caller, never the
subject); revising fixes the subject to the row's and offers save and cancel
instead, with the row's id in the header. The body is one line, not the
console's four rows: GPUI ships no multiline field, and the cut is stated on
the pane rather than hidden. Per-row `edit` renders on the author's own rows
only, queuing the row into the compose above. Filing clears the body and the
subject, saving clears the edit and the body — the console's two landings,
whose statuses this keeps verbatim (`filed #id`, `revised #id`, each
`· reloading reviews`). Anyone-but-author fails with the server's explicit
403, carried on the line rather than pre-hidden on stale data.

One shared helper serves both compose fields (`WriteField::Score |
::Body`) — the fourth copy of the hand-rolled single-line grammar, factored
because the two fields share everything but the slot and the Enter target.
The foot-of-island aside is gone: it said the writes were not carried, which
is now false.

### End session

The console's `z.clock` reduced to its one control, placed where the merge
already lives: a new EXERCISE section at the Step island's tail, below the
disabled run-level verbs and visually separate from them — `End session →`
ends the exercise, not a run, and a live `END SESSION` beside a disabled
`END` would be one label for two verdicts. The island's readouts are the
sim's tick and its pause flag, which this shell has none of, so the merge
carries the verb and names the missing tick rather than faking it. Player
stations never see the section: the console pushes `z.clock` for organizers
only, and the gate reads the same `player_station()` the column does —
extracted to one helper so the two cannot disagree. The binary transitions to
closure and echoes the held game back; the drain holds it through `apply_end`
(a method so a test holds exactly what the drain holds), and the column
recomposes to the debrief around it.

### Verification

| claim | how it was checked |
| --- | --- |
| the bins build | `cargo check --bins` clean |
| the lib holds | `cargo test --lib`: 507 pass, same 4 pre-existing failures |
| the record guards run in order | `record_judgement_guards_run_in_order` (seat, subject, score, trimmed post, write latch) |
| file and revise guard their drafts | `file_and_revise_guard_their_drafts` (empty room, empty body, trimmed post, nothing-queued, empty revision, write latch) |
| the end posts once and refuses players | `end_session_posts_once_and_holds_closure` (single post, busy latch, closure held, player refusal) |
| the proto builds | untouched — no new backend type named — clean |
| RECORD draws | `/tmp/gpui_record.png` (game 4, judge seat): subject pill minus self, score hint, free-text note, live RECORD JUDGEMENT, MARKS + append-only, RELOAD, `No marks yet.` |
| FILE draws | `/tmp/gpui_file.png` (game 4): FILE, both subject pills incl. self, body hint, FILE REVIEW, DOCUMENTS + author-owned, RELOAD + mine only, `No documents yet.` |
| EXERCISE draws | `/tmp/gpui_end2.png` (game 5, `TFG_SCROLL=250`): below the disabled GAME MASTER verbs, the tick note and a live END SESSION → (`TFG_SCROLL=500` overshoots to COMMAND — recorded so the next shot starts closer) |

**Still open.** Groups (no read route), the exercise tick (no clock GET
exists — the verb is carried, the readout is not), island BODY scrolling by
gesture, and the trailing-glyph notdef. `zone_islands(NoSession,_)` still
wants the three-way pick: leave named, drop, or make reachable. Click-proofs
for record, file, revise and end (kernel-tested, code-reviewed, unphotographed
— gestures still do not reach the window) join the helm and the reloads in
that queue. Embark a unit on any dev game and the Embarked read half becomes
the next slice.

## The scenario clock's hold, and NoSession closed (2026-10-14)

Two authorizations, two very different sizes. NoSession closed with a comment
(see below); the hold is the slice.

### NoSession: leave named, with the reason

The question was what "dead" means and what reachability would cost. The
kernel handles the state and the arm is tested — it is dead only in render,
where a signed-out shell draws the login card instead of the column. The
deciding fact is that **the Fleet register needs a token**: a reachable
NoSession would be an empty register beside the card, strictly worse than
the card alone. So the arm stays, and its doc now says reachability waits on
an unauthenticated register read — Minos's question, not this column's. No
code changed, no test touched.

### The hold

The clock survey had shown the shell carrying only the factor write, but
pause and resume are plain POSTs needing no clock channel — the backend's
own contract says the writes ARE the read. Only the tick readout stays out.

**What the shell holds.** The last answer's `running` and nothing else — no
clock GET exists, so there is no fuller read to hold, and the factor readout
stays on the re-read `facts.time_factor`. `None` until the first answer;
unknown attempts pause, the console's rule. A new hold (sign-in, created
game) resets both the clock and the grant: an answer proved the OLD game's.

**The grant latch.** A 403 answers `Denied` distinctly from `Failed` (new
`ClockError`), latches `clock_denied`, and speaks the console's landing
sentence verbatim — shared as `CLOCK_GRANT` so the binary's factor arm names
the same gate. While latched the verb refuses locally instead of spending a
doomed POST. Factor posts stay unlatched by design: they are deliberate
drafts, and a second 403 reports loudly rather than wedging. An answer
clears the denial — it belonged to an older hold or a changed matrix.

**The verbs.** One message, `SetPaused { paused }`, decided shell-side from
the held answer like the console's `pause_or_resume_minos` (held → resume,
else pause). Control draws SCENARIO CLOCK, the RUNNING/HELD readout when
known, and a pill that always asks the other one. The landing keeps the
console's status sentence verbatim (`Scenario clock {verb}: assumed … ·
running|held · accepting orders|orders closed · {factor}x`) and feeds it to
the Log like the console does. The binary maps pause/resume to the two
endpoints, eprintlns the outcome, and refuses signed-out like every arm.

**The box.** The hold joined Control at 182pt and the pill painted half
below the island's own box — the silent-clip defect, caught on capture, not
by any test. Fixed the box (182 → 210, with the doctrine comment), updated
the two pinned stack asserts, and re-shot: the pill fits with room to spare.

### Verification

| claim | how it was checked |
| --- | --- |
| the bins build | `cargo check --bins` clean |
| the lib holds | `cargo test --lib`: 508 pass, same 4 pre-existing failures |
| the toggle follows the last answer | `pause_follows_the_last_answer` (unknown pauses, held resumes, busy latch, denial latches + refuses locally, failure keeps held state, verbatim status) |
| the proto builds | `GameClock` + `GameClockSegment` mirrored in the shim (fifth and sixth shapes — the drift guard firing twice in two slices) |
| Control draws the hold | `/tmp/gpui_clock3.png` (planning hold): multiplier, Apply, SCENARIO CLOCK, full PAUSE pill inside the box. `/tmp/gpui_clock2.png` is the same shot at 182pt with the pill clipped — kept as the defect's photograph. |

**Still open.** Groups (no read route), the tick readout (no clock GET),
island BODY scrolling by gesture, and the trailing-glyph notdef. The grant
sentence is now shared by three paths (pause verb, factor refusal, denial
latch) — if Minos rewords the 403, `CLOCK_GRANT` is the one place. Click-proof
for pause joins the queue behind record, file, revise, end and the helm:
kernel-tested, code-reviewed, unphotographed.

## The Embarked read half, on real data (2026-10-15)

The owner asked for the 404 first, and the 404 gave up everything else with
it. The routes are unchanged (same paths, verbs and `unit_ids` body as the
console sends), the middleware denies with 403 rather than 404, and the 404
`Resource not found` comes from exactly two server lines: no game row, or no
`game_units` row for the host — i.e. the drop named a hull outside the held
game. Reproduced both shapes against the live API (bogus host → 404,
stranger child → 400 with the per-row refusal), then proved the path with a
round trip: embarked KRI Ahmad Yani aboard KRI Abdul Halim in game 5 (201),
watched the ring rule refuse the reverse (400), detached, verified empty —
and re-attached cleared, which is the state the island now reads. The wip
backend changed nothing under this surface.

Two things learned on the way down deserve keeping. First, the server allows
embarking in execution — composition edits are not phase edits, and the 201
landed on G3 mid-execution. What the owner hit is the CLIENT: the drop starts
in the Fleet picker, a planning/preparation surface, so execution offers no
drag source and no mouse path. Second, the register's candidate list was
empty for the host while the game-side attach succeeded — no contradiction:
the console derives its gate from the game's own pieces and complements, not
from the register list.

### What was built (read half only)

**One hull's passengers, lazily, in the modal zone.** `embark_host` draws
the modal (`open_embarked` is the only way in, mirroring the console's
`open_embark`; any other modal clears the host so nothing stale lingers
underneath). The cache is the console's `embarked` map: kept answers are
kept, most hosts carry nothing, and nothing is prefetched. The first draw
issues the read — latched single-flight, so seeding the host seeds the whole
open — and a refusal stops the retry loop on the status line with a Reload
pill the console does without (a failed first load needs a retry path, and
the island it ports has none to copy). Answers file under their own host:
overtaking opens cannot misfile one hull's passengers under another's.

**The rows are the console's verbatim**: name, `hull · class`, and the
clearance in its own words (`cleared — may act`, green the way the pip is
green; `aboard — takes no orders until cleared`). Empty reads `nothing
embarked`. Unembark, clear and revoke stay named, not drawn. The painted
cross closes it — never the glyph, per the parked notdef the bands work
around the same way.

**The doors are the console's per-piece row**: `Embarked … show…` on every
Fleet piece row, naming the count once the cache knows it (`N aboard ·
show…`).

### Verification

| claim | how it was checked |
| --- | --- |
| the bins build | `cargo check --bins` clean |
| the lib holds | `cargo test --lib`: 509 pass, same 4 pre-existing failures |
| open asks once, answers file by host | `embark_opens_once_and_files_under_its_host` (modal-zone exclusivity, busy latch, per-host reads, empty caches as empty, refusal keeps cache, no stale host) |
| the proto builds | `EmbarkedUnit` mirrored in the shim (seventh shape) |
| the modal reads the live passenger | `/tmp/gpui_embark.png` (game 5, `TFG_EMBARK=8`): EMBARKED aboard KRI Abdul Halim, KRI Ahmad Yani + class + `cleared — may act`, writes aside |
| the doors draw | `/tmp/gpui_embark3.png` (planning hold, 2 pieces): `placed show…` on both rows |

Game 5 now carries one cleared embarkation as a standing fixture — placed
and cleared deliberately (an uncleared passenger takes no orders, which
would be a landmine for anyone driving G3), revertible with one detach.
If it ever needs to move, the island reads whichever host holds whom; it
names no game.

**Still open.** Groups (no read route), the tick readout (no clock GET),
island BODY scrolling by gesture, and the trailing-glyph notdef. The
embark WRITES (unembark, clear, revoke) want the owner's word before any
verb is drawn — the aside holds their place. Click-proof for the `show…`
doors joins the queue: kernel-tested, unphotographed.

## The embark writes, and a stray click (2026-10-15)

Authorized as the read half was. Three row verbs, one press per row, in
the console's guards and sentences.

### What was built

**Per-row pills**: `Unembark` always, `Revoke` when cleared else `Clear` —
the console's right-aligned row buttons, where each click pushes one id.
The messages carry the row's own pair (`UnembarkUnit` / `ClearAboard` /
`RevokeClearance { host, unit }`); the shared latch refuses a second verb
while one flies, in the console's `{verb} refused: …` sentences. The
answer replaces the host's contents and stores the capacity warnings
(`embark_over`, new map beside the cache), filed under the answer's own
host like the read. The rows going empty IS the success feedback, so the
line clears; refusals land on it and keep the cache. No force-draft fixup
on unembark, unlike the console: a freed hull was a piece before she
embarked and still is one, so the next session read picks her up with no
local staging. The writes aside is gone — it said they were not carried,
which is now false — and the modal draws the in-flight line plus the
yellow over-capacity lines in the console's sentence.

### Verification

| claim | how it was checked |
| --- | --- |
| the bins build | `cargo check --bins` clean |
| the lib holds | `cargo test --lib`: 516 pass, same 4 pre-existing failures |
| one piece per press, filed by host | `embark_writes_carry_one_piece_each` (wire shapes, shared latch + both refusal sentences, replace + warnings, refusal keeps cache) |
| the proto builds | `OverCapacity` + `EmbarkationResult` mirrored (eighth and ninth shapes) |
| the pills draw | `/tmp/gpui_embark_writes3.png` (game 5, host 8, cleared row): UNEMBARK + REVOKE beside `cleared — may act` |

### The stray click — read this before trusting a capture

The first two captures of this slice showed NO modal, and the second run's
log ended with `unembarked 5 from 8` — a live write posted with no
operator and no test running. Every `send_unembark` call site is a pill
`on_click` or a test-local channel (verified by grep); the tick loop only
drains; nothing auto-sends. The only consistent story is stray input events
reaching the live window: the cursor visibly moves between captures, and
this machine carries real input hardware (`mouse0-2`, twenty event nodes)
whose clicks no harness here synthesizes — the documented "uinput never
reaches the window" is about OUR device, not every device. Across the two
runs the strays closed the modal twice and posted one unembark, which
emptied the fixture; it was re-attached and re-cleared, verified
`[(5, True)]`, and the third capture is clean.

Two standing consequences. First, **the run log is part of every capture
now**: a PNG proves the pixels, the log proves nothing else fired — this
is exactly what caught it. Second, the exposure is every live verb in the
shell, not these pills: helm, record, end session and pause are all one
stray click from posting. The click-proof queue was already blocked on
gestures; it is now also the place where a confirmation policy would be
decided, if one ever is — the console has none, and this slice adds none.

Follow-up diagnosis (2026-10-15): the box carries a Logitech wireless
mouse plus a Synaptics touchpad, no VNC/RDP listeners, no remote input
processes — and this account cannot read `/dev/input` (not in the `input`
group), so the event stream itself is unwatched. Ruled out: our synthetic
device (never registers), remote sessions, in-app auto-send (every send
site is a pill `on_click` or a test-local channel, verified by grep; the
tick loop only drains). Not ruled out: a worn mouse switch bouncing a
full press-release pair, or physical touchpad contact (tap-to-click reads
commented-out in the Hyprland config, i.e. libinput defaults). The
decisive test needs hands: unplug the wireless receiver, rerun any
capture, watch the log — strays gone means replace the mouse.

Confirmed live the next day: across four picker captures, strays opened
the fleet picker unprompted and posted a real `HoldSession` (read-only,
fixture-safe — and incidentally proving the hold flow end to end). Every
capture with visible verbs is now retried until its run log is clean.

## Code review follow-ups (2026-10-15)

Ran the two-axis review (standards + spec) over the recent slices and fixed
the three concrete findings; the rest was clean (0 hard breaches, all
claimed plumbing present).

- **Duplicated resolvers → one helper.** `judge_options`/`review_options`
  and `judge_subject_resolved`/`review_subject_resolved` are now
  `subject_options(exclude_self)` + `resolve_subject(stored, options)`.
- **Invented end-session sentence → removed.** The console's `close_game`
  has no player gate — organizer-only by column placement, server
  arbitrates — so the send carries no seat check either, like
  `advance_execution`. The test now asserts a player station posts and the
  server refuses loudly, instead of asserting a sentence the console never
  says.
- **`TFG_EMBARK` documented** alongside the other seed families in the
  handoff's seed table.

Left standing: the three judgement-call smells (AppMode/Mode name drift,
`Option<bool>` clock, both pre-existing or documented) and the reviewer's
NoSession note, which misreads render-gating for reachability.

## Map drag lag: pump sleeps, not GPUI (2026-10-15)

Reported as laggy pan/zoom with the question whether GPUI is the cause.
It is not: every frame cost 240–400ms warm tiles included, and the split
now logs the parts — `pump 4: 225 + 17 render`. Of ~245ms, ~225 is four
`render_once` + 50ms sleeps "so async work (tiles) can land", ~35 is real
render + readback. Cold frame 252ms vs warm 237ms had already said tiles
cost ~15ms; the split buries the tile hypothesis the rest of the way.

The second find is the actual defect: every gesture sent `JUMP_PUMP` (6)
— Zoom, Pan and Jump alike — although the const's own doc promises the
ordinary gesture spends fewer. A drag step paid six sleeps (~350ms a
frame, ~3fps) for a promise nobody cashed.

The fix is the promise: new `GESTURE_PUMP = 1` for the Pan and Zoom arms
(one render + one 50ms grace — network tiles out-take any sleep budget on
fresh ground, and the cache answers warm ground inside the render), full
budget kept for Jump, warm-up and the config default. Predicted gesture
frame ~70–90ms (~11–14fps) by the same arithmetic the split measured;
gesture frames themselves cannot be produced headlessly here, so that
prediction is code-reviewed, not photographed. No correct seam exists for
a regression test — the pump mapping lives in the binary's arms, where no
unit test has ever run — which is noted, not worked around.

Also fixed while measuring: the capture script still launched
`./target/debug/tfg_gpui`, a pre-cutover fossil, after step 5 moved the
binary. The first perf capture ran the fossil (same sleeps, so its numbers
stand, but it was not the tree). The script now launches
`./target/debug/tfg`, `pkill`s the right process name, and the fossil is
deleted so it cannot recur. Any post-step-5 capture taken before this fix
is suspect and was re-taken where it mattered.

## U8 step 1, third increment — 6 more tests off egui (2026-10-15)

Twelve tests stood in `main.rs`; eight stand now. The inventory held up
almost exactly as written: the five source-checks stay (they read
`include_str!("main.rs")`), the theme/boot halves stay (they paint an egui
`Context`), and the marker hit test stays deliberately — it pins that the
egui PAINTER passes the geometry the kernel produces, which is a claim
about this file's painter, not the geometry.

### Moved, verbatim, with their rules

- **`tfg::phase` (new):** `Phase`, `AppMode`, and both predicates. The enums
  are used 70-odd times across the app's mode machinery, so they travel as
  one import — use sites untouched, the way the markers move worked.
- **`tfg::map_render`:** `zone_outline_hit` + `distance_to_segment`, pure
  tuples all along.
- **`tfg::view`:** the inspector image fit as `(f32, f32)` tuples, with its
  three bounds consts; the three egui call sites convert at the boundary.
- **`tfg::tokens`:** the sizes-over-stock asserts and the scale-over-one
  assert, split off from the theme/boot tests whose other halves paint.

### Deleted

`contrast_ratio` in `main.rs` (egui `Color32`): its test moved to
`tfg::markers` long ago and no caller remained — the Rgba twin there
carries the rule. Found by grep, not by suspicion.

### Verification

| claim | how it was checked |
| --- | --- |
| the lib holds | `cargo test --lib`: 515 pass (509 + 6 moved), same 4 pre-existing failures |
| the bins hold | `cargo test --bins`: 8 pass — exactly the expected remainder (5 source checks, theme, boot, marker hit) |
| nothing outside moved | repo-wide grep: no caller of any moved item outside `main.rs` before the move |

**Still open.** The 8 that remain cannot leave until their surfaces do —
each is now named in the module comment with its reason. U8 step 5 (delete
egui et al) still waits on those surfaces plus the packaging/CI cutover;
this increment moves the count, not the date.

## U8 prep A — the safe half of the cutover (2026-10-15)

Authorized as "no product consequences": move everything deletable
without deleting anything shippable, so step 5 lands as `main.rs` +
friends + deps + CI switch rather than as archaeology.

### Halo out of `fx`

`halo.rs` (92 lines, zero imports) is now `tfg::halo`; the shell reads
`crate::halo` and the five halo tests travel in the file. `fx` keeps a
re-export door (`pub use crate::halo::{…}`) so the console, the examples
and the `halo_seam` probe hold their paths. Both protos updated:
`gpui-island` points at the new file, `p5-epaint` gains the root alias its
`fx` include resolves through. Nothing renders differently — same file,
new address.

### The other console answers in `tfg-gpui`

`--version`, `--check-runtime` and `--probe-gpu`, mirroring the egui
binary's contract line for line. Two supporting moves, both shared rather
than duplicated: `runtime_self_check` (+ its 3-line `validate_map_seed`)
is now `tfg::selfcheck`, parameterized by binary name; the `.env` loader
is now `tfg::paths::load_dotenv`, called in the same two places. The probe
itself names `wgpu` directly (new dep, pinned to the lockfile's 30.0.1)
instead of riding eframe's re-export — with the honesty note attached:
it reports machine capability, and proves the blade path only by proxy.

### Triage, not deletion

- **Examples:** the five U5 probes (`choice/input/miller/tooltip/window`)
  stay. They measure the egui reference the log's numbers came from; they
  die with it at step 5, which is the right time — deleting evidence
  early buys nothing.
- **Mirror test:** `view`'s size mirror now pins literals instead of
  `tokens`, with the old values kept in-test so drift still fails loudly.

### Verification

| claim | how it was checked |
| --- | --- |
| the lib holds | `cargo test --lib`: 516 pass (halo's 5 in their new home), same 4 pre-existing failures |
| the bins hold | `cargo test --bins`: 8 pass; `cargo check --bins` clean |
| the protos hold | `gpui-island` and `p5-epaint` build (p5 needed the wgpu edge for the shared probe) |
| the examples hold | `cargo build --examples` clean |
| the flags answer | `tfg-gpui --version` → `tfg-gpui 0.1.0`; `--check-runtime` → runtime ok (41 classes, 125 units); `--probe-gpu` → DiscreteGpu ready, first frame 694µs, exit 0 |

**Still open.** The egui surface itself: `main.rs`, `chrome.rs`,
`camera.rs`, `picker.rs`, `tokens.rs`, the five probes — all main-only,
all dying together at step 5 with the deps, the lockfile, `about.toml`
and the CI/packaging switch. Nothing in this increment changes what ships;
everything in it shrinks what step 5 has to touch.

## U8 step 5 — egui deleted, `tfg` is GPUI (2026-10-15)

Authorized as the cutover's last step. The product terms (Vulkan
mandatory, halo Low tier, no Xvfb capture) were accepted with ADR-0020;
this entry is the mechanics and the proof.

### Deleted

`src/main.rs` (21k lines, the console), `chrome.rs`, `camera.rs`,
`picker.rs`, `tokens.rs`, `src/fx/` (painters; `halo.rs` already lives at
the root), the five U5 egui probes, and `proto/p5-epaint` (an eframe app
whose includes died with the modules — its U5/U7 evidence stays in this
log). `gpuprobe.rs` survives: it names `wgpu` directly since prep A.

### The 49-test accounting

Lib count falls 516 → 467, all four pre-existing failures unchanged. Every
lost test is classified, none merely missing:

- `tokens` (6), `chrome` (~27), `camera` (~7): egui-type geometry and
  paint assertions. The portable halves left earlier (`phase`,
  zone-pick, image fit, text sizes); what remains tests the OLD
  toolkit's layout, which the shell's own 500+ tests replace rather
  than repeat. Porting them would re-implement egui layout in tuples —
  a port of a dead renderer.
- `picker.rs` (untracked, ~9): never committed, egui-coupled by
  construction, unrecoverable and by the same rule.
- The 8 `main.rs` bin tests die with the binary they tested, per the
  step-1 analysis that named each unmovable.

### The binary kept its name

`src/bin/tfg_gpui.rs` moved to `src/main.rs`: one package, one binary,
still called `tfg`. This is why the CI needs ZERO edits — matrix,
`--check-runtime`, `--version`, `--probe-gpu`, deb and app-bundle paths
all name `tfg` already, and the flags were ported in prep A. `about.toml`
needs nothing: it is an allowlist, still valid. `scripts/ui-shoot` is
retired in place (it drives `./target/debug/tfg` on Xvfb, where GPUI
never presents); `gpui-capture.sh` is the path. PRODUCT.md's two
egui-as-shell statements updated; ADRs and old log entries stand as
history.

### Verification

| claim | how it was checked |
| --- | --- |
| the lib holds | `cargo test --lib`: 467 pass, same 4 pre-existing failures |
| the bins hold | `cargo test --bins`: 0 tests (nothing left to test at that level) |
| the tree builds | `cargo check --bins`, `cargo build --examples`, `gpui-island` all clean |
| no egui remains | `egui`/`eframe`/`egui_extras` gone from `Cargo.toml`; lockfile carries no `egui`/`eframe`/`epaint` package |
| the shipped binary presents | `/tmp/gpui_cutover2.png`: `tfg` on game 5, Operator + Step + Command, live map, `--version` → `tfg 0.1.0`, `--check-runtime` ok (the earlier `cutover.png` ran the fossil — retaken) |

**Still open — and none of it is egui.** Groups (no route), tick readout
(no endpoint), Transcript (local trace, by design), island body gesture
scroll, the notdef (parked), the 9 click-proofs, attach flow (needs a
drag source). The cutover is done; what remains is product work.

## Transcript surveyed, exclusion stands (2026-10-15)

Asked whether the last Assessment tab is portable after all. It is not:
the console's Transcript is the frozen tail (last 200 lines) of the sim
thread's local JSONL journal, read once on Close — and the shell has no
sim thread, no journal files, and no map-ghost replay. Porting the tab
would mean building a journal subsystem, not porting a surface. The
shell's `log_feed` covers the live-line class already (6 call sites
against the console's 36 — the sim/live/order-result lines have no
shell-side source). Exclusion stands, now on inspected evidence rather
than inherited lore.

## The session picker: login lands on choice, not mid-execution (2026-10-15)

Reported as "first login shows an execution view already". The data was
all correct — SUEX is Super User's call-sign in game 44, the 2000 stamp
is the server's own step time, the seat and hull are really held — but
the HOLD was list-order luck: five Commando games, no choice offered, the
first player game in server order wins. The console never does this;
login lands on `z.start` and the operator picks.

### What was built

**`z.start`, ported as a column island.** `SessionRow` (id, name, state,
seat role, judge side — a display projection like `MyShip`, in server
order) rides the sign-in answer beside the roster; the NoSession column
is now Operator + Sessions + Fleet. Rows show name bright with state and
seat dimmed (`Commando`, `Penyelenggara · judge`, `not seated`); the row
IS the verb. Holding replaces the session whole through the existing
`create` feed (`HoldSession` → pinned `fetch_session` → `apply_hold`),
so the column recomposes around the held game — proven in-harness from
picker (`operator/sessions/fleet`) to planning column.

**Card sign-in stops auto-holding.** The sign-in answer carries roster,
games and an unheld session; token kept, game id zero. Env-auth startup
is untouched (pin or preference holds as before) — only the card path
changes, which is the reported one.

### Verification

| claim | how it was checked |
| --- | --- |
| the bins build | `cargo check --bins` clean |
| the lib holds | `cargo test --lib`: 468 pass, same 4 pre-existing failures |
| the kernel holds | NoSession arm `[Operator, Sessions, Fleet]`, key names itself, heights/duplicates loop-clean |
| hold names the pick | `hold_names_the_picked_game` (picker column unheld → `HoldSession{9}` → busy latch → `apply_hold` installs + list survives → planning column) |
| the picker draws | `/tmp/gpui_picker2.png` (`TFG_SESSIONS=1`): NO SESSION operator, 3 SESSIONS rows with real names/states/seats, register below |
| fixture honesty | seed rows use real game names, states and seats (44/38/9 read live); only the hold is staged |

## Seat-me pill, photos on sign-in, rectangular thumbs (2026-10-16)

### Why creating a game seats nobody

The server does not seat the creator — a fresh game's participants list
is empty, and neither client ever seated on create (the console's
`apply_created_game` just holds). So the seat has to be taken, and the
picker refused drops with no way forward. Now the force section offers
it in one click when the room holds no seat for the operator and the
game defines an organizer role ("Game Master"/"Penyelenggara", the
console's constants, matched case-insensitively): "seat me as …"
stages exactly what the Players island would — same `stage_seat`, same
name-derived call sign — and the seatless drop refusal points at it.
Auto-seating on create was considered and refused: joining a room is
visible to everyone in it, and a client that joins you silently is the
consent the pill keeps. No organizer role, no pill — seating stays the
Players island's job, never a guessed role.

### Photos also fetch after a card sign-in

The glyph fallback in the report was a startup-only fetcher: a launch
without credentials fetched nothing, and the later card sign-in never
started one. The fetch is now a function of (host, token, channel),
run at startup when a token exists AND after every sign-in on the new
token. The shell keys by unit id and skips what it holds, so a second
pass re-fetches but never repaints.

### Thumbnails leave the circle

56×38 rounded rects at the photos' own aspect, covered never
letterboxed, fetched at 192px so the shrink stays ahead of the paint —
one shared face helper for the row and the ghost, so the hand never
swaps pictures. The glyph fallback lives in the same rect. The disc
helper is deleted, not deprecated.

### Verification

| claim | how it was checked |
| --- | --- |
| the lib holds | `cargo test --lib`: 489 pass, same 4 pre-existing failures; `--bins` 2 pass; bin, examples clean |
| organizer resolves, or doesn't | `organizer_role_{matches_the_deployments_words,absent_without_the_words}` |
| the pill seats | `seat_me_stages_the_organizer_seat` (one Seat write, own seat, name-derived sign) and `seat_pill_seats_from_the_picker` (swept click posts the roster plan) |
| it draws | `/tmp/gpui_fleet10.png` (clean run, log checked): five rectangular photo thumbs on the Sea drill, verbs intact, fetch pass done for all 62 pictured hulls |

## Placing IS assigning (2026-10-16)

The 404 in the screenshot was the server being literal: placement is a
per-piece row, and a register hull dragged straight to the map was never
added to the exercise — no piece, no row, 404. The first fix (refuse
locally with "add it first") was wrong too: it defended the two-step
model the operator never asked for. Dropping a hull on the map assigns
it — one gesture, `DeployAssign` carrying the operator's own seat (the
add button's default; seatless refuses locally, same sentence), with the
binary sequencing `assign_unit` before `set_placement` and answering the
usual re-read session. The host-side twin, `EmbarkAssign`, chains the
attach the same way. Either step names its own failure, and an
assigned-but-unplaced hull is a good state: the next drop places it
outright. Click-to-place stays strict (its arm was struck on a drafted
row; a hull that left since is refused, not silently re-added).

### Verification

| claim | how it was checked |
| --- | --- |
| the lib holds | `cargo test --lib`: 485 pass, same 4 pre-existing failures; `--bins` 2 pass; protos, bins, examples clean |
| drop assigns then deploys | `dragging_a_leaf_row_to_the_map_deploys_it` (now seated, unassigned): `DeployAssign{13, commander:7}` with the drop's fractions |
| seatless drops post nothing | `dropping_without_a_seat_posts_nothing`: empty sink, "no seat" on the line |
| chains, both sides | `dropping_a_register_hull_{assigns_and_deploys, on_a_host_assigns_and_embarks}` direct on `drop_unit` |
| taken paths unchanged | `place_button_arms_and_map_click_deploys`, `dropping_a_leaf_row_on_a_host_embarks_it` still green |

**Still open.** A watched live drop (assign-and-place against the server),
the disk photo cache, hover-truncation on the other modal cards.

## Fleet picker follow-ups: photos, no-drag-map, hide-while-dragging (2026-10-16)

Three complaints, three slices.

### Photos, not glyphs

The glyph was never the design — the console's `unit_thumbnail_ui`
prefers the hull photograph and falls back to the symbol, and the/Minos
side already serves it (`unit_image_manifest` + presigned `hull_image_url`,
with `visuals.rs` holding the rules). The GPUI shell just never fetched.
Now a background thread at startup reads the manifest (62 pictured on
dev), resolves one presigned URL per pictured hull, decodes and downscales
to 96px, and trickles `ThumbRaw` bytes over a channel; the tick mints one
`RenderImage` per arrival through the same raw path the map frames take,
and the rows fill in progressively while the glyphs hold. Photo wins,
glyph stays the fallback, a hull with no picture costs no request. Memory
only — no disk cache yet, so every launch re-fetches; the manifest
version is the obvious cache key when that gets built.

### Dragging dragged the map too

Real defect, harness-pinned: GPUI delivers a press to EVERY hovered
hitbox down the chain, and the full-window map surface is always in it —
the picker card had no hitbox of its own, so pressing a leaf row ALSO
started a map pan, and every drag move panned with the ghost. The card
now truncates hover (`block_mouse_except_scroll`, the island edge's own
rule, so the columns keep scrolling). `pressing_the_picker_card_does_not_
pan_the_map` failed before, passes after. Same bug class lives on every
other modal card — said plainly so it gets fixed deliberately, not
discovered again.

### The picker hides mid-drag

The tick polls `has_active_drag` once per frame into a `dragging` flag
(the drop and ghost live in GPUI's own drag state, which no element
callback can read into the view), and the fleet branch renders only when
not dragging. Press, lift, and the modal is gone — the whole map is the
drop zone; release or cancel and it returns, still drilled where it was.
`dragging_hides_the_picker` pins the flag; the hide itself is three lines
of wiring, reviewed, for the user's live pointer to confirm.

### Verification

| claim | how it was checked |
| --- | --- |
| the lib holds | `cargo test --lib`: 482 pass, same 4 pre-existing failures; `--bins` 2 pass; protos, bins, examples clean |
| bytes mint and refuse | `thumb_image_{mints_from_bytes,refuses_torn_bytes}` |
| card eats its own presses | `pressing_the_picker_card_does_not_pan_the_map` (red before the block, green after; drag/drop/place proofs still green) |
| drag hides, drop returns | `dragging_hides_the_picker` |
| it draws | `/tmp/gpui_fleet9.png` (clean run, log checked): five photo thumbnails on the Sea drill, verbs intact; `/tmp/gpui_fleet8.png`: paint unchanged by the hit-test-only card block |

**Still open.** Disk photo cache (manifest version as key), the same
hover-truncation on the other modal cards, and a live watched drop —
`set_placement`/`attach_units` remain harness-proven to the sink only.

## Fleet picker: drag to deploy, drop to embark (2026-10-16)

Leaf rows drag. A press that moves arms a `UnitDragPayload` (numeric id,
name, glyph) with a thumbnail-and-name ghost; a release over bare map
deploys there (`DeployUnit` with view fractions, unprojected at the
binary's camera through `set_placement`), and a release within 20 map px
of a placed piece's marker embarks aboard it (`EmbarkDrop` through
`game_attach_units`). Both answers re-read like every other write — the
deploy on the shared feed under its own `deploy_busy` latch, the embark
on the embark feed like the unembark/clear/revoke verbs. Click-to-place
covers the pointer-impaired path: a taken row's PLACE button arms the
hull, the map shows a crosshair, and a click that stays put deploys (a
press that moves past 6px pans like any other).

Three things the build taught:

1. **Hover is not topmost.** The drop fires wherever the map is *hovered*,
   and hover truncates only at `BlockMouseExceptScroll` — so a release
   over the picker still drops on the map underneath. The harness sweep
   proved it by accident: a probe's release over the leaf rows posted a
   deploy. Harmless here (a drop is a drop wherever the map shows
   through), but a drop that must NOT fire under chrome would need its
   own truncating hitbox.
2. **A probe release is a drop.** The sweep that finds the drag source
   fires a real drop on the way, and that deploy re-renders the picker's
   status line, shifting every row below the press that found it. The
   drag proof runs sweep-and-drop in ONE pass — the press that starts a
   drag keeps going to the drop — because a second press would land on
   moved rows. Same class as the island-edge defect: geometry read before
   a write is stale after it.
3. **Sweep clicks have side effects.** The place-button sweep clicked the
   exercise's REMOVE and un-drafted the hull sought, then walked past a
   button narrowed to ~24pt. The sweep seeds a fixed status (geometry
   holds still), starts below the exercise section, and steps 6pt across
   the button's end. All of which the test's comments own, because the
   next person to touch those rows will otherwise re-learn it.

Layout: the leaf row stacks (identity line, verbs line). A single line
cannot hold a 30-character name plus two pills in a 252pt leaf, and no
truncate/w_full/flex combination in this toolkit shrinks the text past
its minimum — three captures watched ASSIGNED slide off the modal while
the name stayed whole. Taller rows, verbs always aboard; the console's
46pt pitch had 232pt of leaf to spend and this modal does not.

Not ported: the `All(N)` tab, whole-tree search, photo thumbnails (glyph
discs hold), and the embark capacity pre-checks (the server evaluates;
the answer carries what it said). Deploy/embark against the live server
are harness-proven to the sink only — no capture drives a real
`set_placement` or `attach_units`, by the standing rule against write
verbs on live games. The first real drop wants watched hands, and the
picker's verdict lines are where its refusal will read.

### Verification

| claim | how it was checked |
| --- | --- |
| the lib holds | `cargo test --lib`: 478 pass, same 4 pre-existing failures; `--bins` 2 pass; protos, bins, examples clean |
| the tree builds | bin-side `builder_{walks_group_to_hull,empty_store}` on an in-memory mirror (moved out of the shell — the parity proto stays sqlite-free) |
| the drill walks and clears | `drill_{defaults_to_first_tab,walks_to_the_hulls,parent_change_clears_below}` |
| drag deploys | `dragging_a_leaf_row_to_the_map_deploys_it`: sweep finds the source, one pass to bare map, `DeployUnit{13}` with the drop's fractions |
| click places | `place_button_arms_and_map_click_deploys`: sweep arms `pending_place`, map click posts `DeployUnit{13}` |
| drop embarks | `dropping_a_leaf_row_on_a_host_embarks_it`: marker under the drop, `EmbarkDrop{host:8, child:13}`; plus `drop_{names_the_nearest_placed_host,ignores_unplaced_and_mapless}` |
| it draws | `/tmp/gpui_fleet7.png` (clean run, log checked): Sea drill to 5 hulls, glyph thumbnails, ASSIGNED + PLACE on the taken row, ADD elsewhere |

## Fleet picker: miller columns, thumbnails (2026-10-16)

The flat register list is now the fallback, not the picker. With a synced
mirror the picker drills Asset Group > Category > Type > Class > hull off
the same `store::tax_*` readers the egui console used, with the readers'
own live-hull counts on every tab and row — a tab's number and the rows
behind it agree, because they are the same query. Each leaf row carries a
28px disc with the taxonomy glyph the map would draw (`MAP_INK` on
`PANEL_SLATE`, painted through `canvas`), the port of the console's
`unit_thumbnail_ui` fallback rung. Photo textures stay out: there is no
image pipeline in this shell, and a glyph thumbnail is the honest one.

Plumbing: the binary builds the tree once at startup
(`fleet_taxonomy_from_store`, best-effort per rung — a bad row empties its
rung, never the picker) with each leaf's symbol resolved through the same
resolver the markers use, and hands it to `run()` beside the roster. Drill
state is four `Option<i64>` with first-tab default; a parent change clears
below, the console's rule, through `select_*` helpers. The modal widens to
700 (3 × 128 + flex leaf) — four columns never fit the 520 form, the same
reason the egui picker ran 980.

Deliberately not ported: the `All(N)` tab (tabs are the groups; a union
tab is a merge the drill does not need) and whole-tree search (no query
field in this shell yet). Both noted, neither owed by this request.

Drag is still out — the doc on the picker says so, pointing at the drop
slice next. A row that looks draggable with no map drop is still a control
that does nothing.

### Verification

| claim | how it was checked |
| --- | --- |
| the lib holds | `cargo test --lib`: 474 pass, same 4 pre-existing failures |
| the tree builds | `fleet_taxonomy_{walks_group_to_hull,empty_store}` on an in-memory mirror |
| the drill walks and clears | `drill_{defaults_to_first_tab,walks_to_the_hulls,parent_change_clears_below}` |
| it draws | `/tmp/gpui_fleet2.png` (clean run, log checked): Air/Land/Other/Sea(124) tabs, breadcrumb to Anka-S, four washed columns, glyph thumbnail + ADD on the leaf |

**Still open.** Refreshing the list after a create (a composed game
appears only on next sign-in — noted, not built), and the remembered-game
default from the proposal (deferred: the picker solves the complaint,
persistence is a convenience). The click-proof queue and the mouse
question are unchanged.

## Row clicks were dead everywhere: the island edge (2026-10-15)

Reported as "can't click any session". Reproduced in-harness on the first
try: a computed row-centre click lands and nothing reaches the sink — and
a 16-click sweep of the whole island height lands nothing either. Not
geometry. Then the sweep's negative generalized: no column-island click
has EVER been proven, in-harness or live. The three `simulate_click`
tests in the tree all target the roster SHEET; every island-body verb
(tabs, pills, fields, step declare, helm steppers, these rows) was
sink-tested only. The modals worked because they have no island edge.

Root cause, read off GPUI 0.2.2's own `window.rs`: every mouse listener
requires its hitbox to be hovered, and hover truncates at the topmost
`BlockMouseExceptScroll` hitbox. The island edge — full-panel, painted
LAST, topmost — truncated hover at itself, so no body control under it
could ever fire. The code comment claimed the opposite, porting egui
reasoning where paint-only rims don't hit-test. In egui the rim is
invisible to clicks; in GPUI it was a full-panel click black hole.

The fix is tree order: the edge paints FIRST (under the content), the
close stays last. The 1px border sits at the panel bounds where content
never paints, so the reorder is invisible — verified on capture — and
the edge still shields the map in the gaps, which is its whole job. Both
comments rewritten to state the rule instead of the wish.

`clicking_a_session_row_holds_it` now clicks a real row and asserts the
hold — the first harness-proven island click in the tree, and the
regression pin for the whole class. The click-proof queue just got
shorter the honest way: every island verb is now *capable* of being
driven; proving each one still wants a working pointer.

### Verification

| claim | how it was checked |
| --- | --- |
| the lib holds | `cargo test --lib`: 469 pass, same 4 pre-existing failures |
| row clicks fire | `clicking_a_session_row_holds_it` (failed before the reorder with an empty sink, passes after) |
| the border survives | `/tmp/gpui_picker6.png` (clean run, log checked): picker rows, island edges intact, no paint change |

## The seat-me pill seated judge-side; drops now refuse locally (2026-10-16)

Reported with a screenshot: after the seat-me pill shipped, clicking ADD
on a register hull answered `HTTP 400: id_commander: a judge-side
participant does not command units or task groups`. The user did nothing
wrong — the pill offered the organizer role ("Penyelenggara"), which is
judge-side, and the server refuses judge-side commanders outright. The UI
seated them into a seat that cannot command: a local rule mismatch, fixed
at the seat selection, not at the error message.

The fix, in three parts. (1) `organizer_role()` became two readers:
`player_roles()` (the non-judge roles — what the pills may offer) and
`own_steers()` (`None` unseated, `Some(steers)` from the held role's judge
flag). (2) The picker's force section renders one pill per PLAYER-side
role — the organizer pill is gone entirely; seating as organizer remains
possible on the Players island. (3) `drop_unit` refuses a judge-side drop
locally, after the seatless gate and before the round trip: "your seat is
judge-side — it directs, it does not command; take a player seat on the
Players island". The seatless sentence now points at the player pills when
the game defines any ("take a player seat above").

The ADD button path hits the same server rule — the pill is the fix, not
the workflow: proper testing is to seat as a PLAYER-side role (the pill
now offers exactly those), then drag a hull to the map, and to confirm a
judge-side seat refuses locally with guidance instead of a 400.

### Verification

| claim | how it was checked |
| --- | --- |
| the lib holds | `cargo test --lib`: 490 pass, same 4 pre-existing failures |
| the pills exclude the judge side | `player_roles_{exclude_the_judge_side,empty_when_every_seat_judges}` |
| the pill stages a player seat | `seat_me_stages_a_player_seat`: one write, role_id 1 (was 9), name-derived call sign |
| the pill seats from the picker | `seat_pill_seats_from_the_picker`: click sweep, first roster plan is the pill's |
| judge-side drops refuse locally | `dropping_from_a_judge_side_seat_posts_nothing`: empty sink, the refusal sentence on the line |
| it draws, empty room | `/tmp/gpui_pill.png` (`TFG_GAME=45 TFG_FLEET=1`, clean run, log checked): one pill — "SEAT ME AS COMMANDO" — no organizer pill |
| it draws, seated room | `/tmp/gpui_seat.png` (`TFG_FLEET=1`, clean run, log checked): no pills (the room has seats), operator seated Commando — player-side |

**Still open.** The judge-side refusal on the TAKEN path (re-deploying a
hull already in the exercise) is unguarded — a piece cannot reach a
judge commander through the assign path the server refuses, but a role
change after assignment would. The pill visual in a room with several
player roles (many pills stacked) is untested beyond the sweep. The
`proto/gpui-shell` proto fails to build in this checkout: its
`include_bytes!` references `p5-epaint/assets/`, which is absent —
pre-existing, unrelated to this slice.

## Row thumbnails go full-width: the photo banner (2026-10-16)

The fleet picker's rows carried the hull photo as a 56×38 stamp beside
the name. The user asked for the thumbnail's full width — the photos are
the picker's reason to fetch at all, and a stamp threw the ship away.

The row now stacks: the photograph as a **full-width banner** (236×96, a
2.5:1 strip) across the top, then the identity line (name over sub), then
the verbs right-aligned. The drag ghost keeps its small 56×38 stamp — a
cursor companion is not a banner. `thumb_face` became `thumb_box`
(fixed width, height, glyph scale) with two wrappers: `thumb_face` for
the ghost, `thumb_banner` for the row.

**The GPUI 0.2.2 lesson, paid for once:** the banner was first written
`w_full()` (percentage width on a flex item) and it painted NOTHING —
neither the photo nor the glyph fallback — and the row's verbs vanished
with it (the card's layout broke and they fell below the scroll fold).
A fixed width (`LEAF_CONTENT_W` = 236, the leaf column 244 minus the
card's padding, all derived from the fixed 700×580 modal) paints
everything. Percentage widths on flex items resolve their children's own
percentages against nothing in this toolkit; fixed widths do not. The
constant's comment records the arithmetic and the trap.

### Verification

| claim | how it was checked |
| --- | --- |
| the lib holds | `cargo test --lib`: 490 pass, same 4 pre-existing failures |
| it draws | `/tmp/gpui_banner2.png` (clean run, log checked): full-width photo banners, name + sub under each, ASSIGNED/PLACE/ADD right-aligned |
| the `w_full` trap is real | `/tmp/gpui_banner.png` (broken): empty banners, missing verbs — bisected by the stash-capture-compare pass |

**Still open.** The banner height (48) suits the archive's wide
panoramas (measured: 5.9:1–9.4:1, mode 9.4:1) — a fixed box can't match
every aspect, so `Contain` letterboxes the outliers (a tank, a
helicopter, a drone — 3 of 62) rather than cropping them; revisit if
those read too small. The flat (fixture/offline) register rows stay
text-only.

### Follow-up: whole body, not a zoomed slice (same day)

The first banner used `ObjectFit::Cover`, and the user's screenshot
showed the defect immediately: the archive's photos are wide whole-body
panoramas (measured off the manifest — 5.9:1 to 9.4:1, e.g. the LPDs
at 1346×227), and the 2.46:1 cover-cropped strip showed a zoomed-in
slice of the hull, not the ship.

`Contain` fits the entire image. A wide photo's contained height is
its width over its aspect — independent of the box — so the banner
also slimmed from 96 to 48 (a 4.9:1 strip the common panoramas
nearly fill) instead of sitting 65% matte. The drag ghost keeps its
56×38 `Cover` stamp: a cursor stamp wants a slice, not a letterboxed
sliver.
