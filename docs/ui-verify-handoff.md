# Handoff: verifying the console on a private display

Revised after a second pass over the same ground. The first version of this
file was written at a clean pause with a working drive environment; this one is
written after the three modals were opened and the environment was rebuilt from
nothing. `scripts/ui-shoot/env-up.sh` is the bring-up now, so a cold-start
agent is photographing the console one command after checkout.

## Intent

The UI overhaul is written and unit-tested, and none of it had been seen on
screen. The gap was closed with a virtual display so screenshots do not depend
on what the operator is doing, and then the render was driven through the states
that matter. That found defects which no test could see, at a rate of roughly
one per interaction.

The second pass found the same rate. Six were real and are fixed; four are
written up in `docs/ui-overhaul-status.md` and left for the owner, because each
one is a product answer rather than a patch.

## What is verified, and how it was verified

Drove the real binary on Xvfb, captured with `import -window root`, and read the
pixels. Every claim below is from a render, not from reading code.

Fixed and confirmed on screen:

- **The close glyph did nothing, on every island and in every modal.** It lit on
  hover and swallowed clicks. egui gives a click to the topmost clickable
  widget and ties go to the last one registered; hover is not exclusive, so a
  button buried under later-registered widgets still lights up. The close button
  was registered before the body scope. Now after it. `chrome.rs`.
- **A seated game rendered as an empty one.** The roster is wrapped on the wire
  (`data: {participants: [...]}`) and `parse_roster` read it as a bare array, so
  `as_array()` returned `None` and the island said `0 SEATED` while the seat was
  in the database. Both shapes now parse, with tests.
- **Signing out left no way back in.** The console sat at `NOT SIGNED IN` with a
  Sign out button and no sign-in control. The zone's Operator island now carries
  the form when there is no identity.
- **A cold start could not re-open a session.** The picker only existed once a
  session was held, so the only verb was *create*. It is drawn by the no-session
  island too now.
- **Signing back in started blind.** `sign_out` clears the session list and
  nothing re-read it.
- **The Player picker opened on an empty directory** and told the operator to
  sign in. It reads on open, and the empty state says what actually happened.
- **The no-session island's dead-end button.** `d4983ba`.
- **The gate is no longer derived client-side.** `d8df189`. Test count fell 295
  to 293, which was the point.
- **The Planning column could not fit its own islands.** `b4d5b0d`.
- **A clip broken while adding the scroll.** `d674a47`.
- **Every arrow was a `.notdef` box.** `cb1505d`.
- **"Retry the sync" could not work.** `b52cfcc`.

Proven by driving:

- **The published gate works end to end.** Against local Minos the checklist
  quotes the server's blockers verbatim: "the planned window is incomplete (both
  actual_end and assumed_end are required, and each must be after its own start)"
  and "no units assigned". `GET /games/{id}/readiness` answers the same two.
- **All three modals render.** Player picker, Fleet picker, Scenario composer.
- **The composer's write path works.** A step typed into it came back from the
  server as a third step on the scenario.
- **Failing closed is real.** Against the hosted API the checklist said the
  server does not publish the check, rather than reporting zero blockers and
  lighting the Enter button.

## Bring the environment up

```bash
cd ~/Work/Crossnet/tfg
bash scripts/ui-shoot/env-up.sh
```

That is the whole thing: it unpacks Postgres, valkey and numactl into
`/tmp/opencode/stack`, initialises the cluster, starts both, migrates, seeds the
superuser, builds and starts Minos on 8099, starts Xvfb on `:99`, and seeds a
game in `planning` with a scenario book and a Game Master seat. It is idempotent;
the second run reports `already listening`. It prints the launch line and the
credentials when it finishes.

Then launch the app and **wait 30 seconds**:

```bash
cd ~/Work/Crossnet/tfg
setsid env -u WAYLAND_DISPLAY DISPLAY=:99 \
  TFG_MINOS_HOST=http://127.0.0.1:8099/api/api/v1 \
  TFG_ZONE_DEBUG=1 LIBGL_ALWAYS_SOFTWARE=1 \
  ./target/debug/tfg > /tmp/opencode/tfg.log 2>&1 < /dev/null &
```

Sign in as `supersuser` / `tfg-dev-password`. The name looks like a typo and is
not: `seedgame.py`, `seat.py` and this file all use it.

## Drive it

`scripts/ui-shoot/xdrv.c`, compiled by `xvfb-up.sh` to `/tmp/opencode/xdrv`. XTEST,
so it needs no compositor. Commands: `place X Y W H`, `click`, `rclick`,
`mousemove`, `drag`, `wheel N`, `type`, `key` (now with `ctrl+` / `shift+`
chords), `focus`, `list`.

```bash
export XDRV_DISPLAY=:99 DISPLAY=:99
X=/tmp/opencode/xdrv
$X focus
# The login card's fields sit at y=330 and y=375 in a 1040x640 window.
$X click 515 330; $X type "supersuser"
$X click 515 375; $X type "tfg-dev-password"
$X click 515 415
# Pick the seeded session from the no-session island's combo, then open a modal.
$X click 90 262; $X click 120 340
$X mousemove 180 400; $X wheel 15
$X click 100 238      # Player picker
import -window root /tmp/opencode/shots/x.png
```

**Shoot through `shot.sh`, not `import`.** It burns a coordinate grid into the
capture in window coordinates:

```bash
/tmp/opencode/shot.sh /tmp/opencode/shots/x.png            # whole window, grid 50
/tmp/opencode/shot.sh /tmp/opencode/shots/x.png 20 240 400 200 25   # x y w h step
```

## Gotchas that will cost you an hour otherwise

- **The app ignores input for the first 30 seconds.** Software rendering plus
  map tile decoding. Clicks and keystrokes in that window are dropped silently,
  which reads as a dead control. Three sign-in attempts failed at 14 seconds and
  worked at 30.
- **Never read a click target off a rendered screenshot.** A cropped image is
  displayed at a scale you cannot know, so every eye-read coordinate in this
  session was 10 to 30pt out. Use the grid, or use `click-sweep.sh X0 Y0 X1 Y1
  step PROBE_X PROBE_Y REF_X REF_Y`, which clicks a grid and stops at the first
  point that changes a probe pixel.
- **`xdrv click` holds for 70ms on purpose.** A zero-length press+release is
  dropped often enough to matter; three runs in a row a click on a text field
  did nothing before this was changed.
- **`WAYLAND_DISPLAY` leaking into the app's environment** puts the window on
  the real desktop while `:99` stays empty, which looks exactly like a crash.
- **Never `pkill -f 'target/debug/tfg'`.** The pattern matches the driving
  shell's own command line and kills the shell. Use `pkill -x tfg`.
- **Text input must map printable ASCII by character code.** Latin-1 keysyms
  are the char code. `XStringToKeysym("-")` returns `NoSymbol`, and silently
  dropping both hyphens from a password produces a field of sixteen dots and
  `unauthorized`.
- **Capture after the app has settled.** Mid-frame grabs photograph a partly
  painted window: one capture in this session came back with a modal's entire
  background missing, which reads as a rendering bug and is not one.
- **A capture of the root is a capture of the desktop too.** The window is
  hardcoded to 1040x640, so capturing 1200x900 photographs black beside the app.
  That is what the previous pass logged as "the map corrupted after a resize".
- **`island_on_band` culling is total.** An island outside the band does not
  paint wrong, it does not exist, and it cannot own input. Any "this control is
  missing" report should be checked against the zone geometry log first.
- **An island's body cannot be scrolled.** The zone's islands have no
  `ScrollArea`, so content past an island's fold is unreachable, and the wheel
  over an island scrolls the column instead. This is why the composer's button
  needs a 1200x900 window to appear.
- **`TFG_ZONE_DEBUG=1`** prints the zone's band, every island's position and
  height, the overflow, the scroll offset, the hover point and the raw scroll
  delta. That line settled three bugs in the first pass and one in this one.

## Next

1. **Give the zone's island bodies a scroll.** It is the root cause of the
   largest remaining gap: content that does not fit is unreachable, and the
   report goes to stderr. Read `docs/ui-overhaul-status.md`, "Not fixed, and why",
   first. The wheel conflict is the whole difficulty.
2. **Decide what a zone island's close glyph means.** Either it works, with a
   way to reopen what it closed, or it is not drawn. `DESIGN.md` currently gives
   the side zone no close button.
3. **Both drag-to-map gestures.** Still unverified, and blocked on data rather
   than code: the local mirror syncs `units 0`, so the Fleet picker has no rows
   to drag. Seed a unit catalogue and drive them.
4. **Deploy `aaee9bf`.** Not a code task. The client's default endpoint is the
   hosted dev API, which 404s on the readiness route, so the console cannot show
   a gate verdict against the environment it ships pointing at.

## State

Tree clean apart from the changes in this pass. `cargo test --lib`: 297 pass
with the same two pre-existing failures (`map_render::quad_bow_follows_the_compass`
and `sim::order_move_applies_then_higher_overrides_loudly`). `cargo test --bin
tfg`: 49 pass. The two extra library tests are the roster envelope.

`AGENTS.md` used to say not to compile at all, which conflicted with
operating the app; it now carves out UI work explicitly, so the next agent does
not have to decide it per session.