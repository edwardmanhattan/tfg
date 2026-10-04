# Handoff: verifying the console on a private display

Revised after a third pass over the same ground. The first version of this file
was written at a clean pause with a working drive environment; the second after
the three modals were opened and the environment was rebuilt from nothing. This
one is written after the zone's scroll was fixed and the harness grew the ability
to find a control rather than be told where it is.

`scripts/ui-shoot/env-up.sh` is the bring-up, so a cold-start agent is
photographing the console one command after checkout, and
`scripts/ui-shoot/verify-scroll.sh` is the scroll regression, so the next agent
does not have to rebuild it.

## Intent

The UI overhaul is written and unit-tested, and none of it had been seen on
screen. The gap was closed with a virtual display so screenshots do not depend
on what the operator is doing, and then the render was driven through the states
that matter. That found defects which no test could see, at a rate of roughly
one per interaction.

The second pass found the same rate. Six were real and are fixed; four are
written up in `docs/ui-overhaul-status.md` and left for the owner, because each
one is a product answer rather than a patch.

The third pass found the item the second pass had called structural and
unfixable. It was two defects stacked, and removing both is a deletion. It also
found a seventh dead end, the composer's close, which is the reason this file now
says the harness asserts things rather than photographing them.

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
- **Island bodies now scroll, so nothing in the zone is out of reach.** The
  second pass filed this as structural. It was bodies clipping plus the zone
  growing each island to fit its content, and the growth was what made Essentials
  624pt tall against a 640pt window — an island taller than the screen cannot be
  scrolled to either. Both are gone; the declared height is the footprint again,
  which is what `DESIGN.md` has always said. The wheel is chained, so a body
  takes the gesture while it has room and the column takes it otherwise.
  `1f96f1e`.
- **The composer's close left the modal on screen.** `composer_modal` cleared
  the selected scenario and the draft but never set `composer_visible = false`,
  so the modal re-rendered empty with no way out. Found by driving, because the
  harness clicks the ✕ and then asks whether a modal is up.
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

`drive-to-session.sh` does the rest — launches the app, waits, signs in,
picks the seeded session, and puts the session back into `planning` first:

```bash
bash scripts/ui-shoot/drive-to-session.sh
```

Do not launch the app by hand. The manual line still works and is in the header
of `env-up.sh`, and there are two reasons not to use it: the script resets the
session state, and it never inherits `DISPLAY`.

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

**Let the harness find controls.** `find-buttons.py shot.png [ox oy]` reads a
button's fill out of the frame and returns one row per button, widest first:
`y0 y1 x0 x1 w h cx cy`. `verify-scroll.sh` uses it, along with the island
geometry the app logs, so no click target is ever a remembered coordinate.

This is not a convenience. The first version of `verify-scroll.sh` hard-coded
`(101,520)` for the composer's button; the app was in a different state by then,
that coordinate was *declare ready*, and the harness POSTed the session from
planning into preparation while measuring it. Every later assertion was then a
true statement about the wrong screen.

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
- **There are two scrollbars in the zone and one wheel.** An island body
  scrolls inside its own rect, and the column scrolls as a whole. The chain is:
  the body under the pointer takes the gesture while it has room in that
  direction, and the column takes it otherwise. `verify-scroll.sh` asserts that
  per frame, over the numbers the app printed.
- **Body scroll offsets survive between runs.** They live in egui's memory under
  the island id, so a case that starts without rewinding its body inherits the
  previous case's scroll and then asserts against it. `rewind_body` and
  `reset_column` in `verify-scroll.sh` loop until the offset reads zero, not for
  a fixed number of notches.
- **Never `pkill -f` anything the harness launched.** See below. `drive-to-session.sh`
  once opened the app on the operator's real desktop by inheriting `DISPLAY`.
- **`TFG_ZONE_DEBUG=1`** prints the zone's band, every island's position, size
  and body measurement, the overflow, the scroll offset, the hover point, which
  island's body was under the pointer, and the raw scroll delta. That line
  settled three bugs in the first pass, one in the second, and every assertion
  in the third.

## Verify the scroll without driving anything by hand

```bash
bash scripts/ui-shoot/verify-scroll.sh
```

Seventeen checks, and it exits non-zero on any failure. It brings the environment
up, drives to a known state, and asserts: the wheel over a scrolling body leaves
the column alone; the composer button comes into reach by scrolling the body; it
opens the composer and the ✕ closes it, with no session state changed either
time; the wheel over a gap moves the column; the wheel over a body that fits
moves the column; the wheel over the map moves neither; and at no frame did the
column and a body both take one gesture.

The last one is the invariant, and it is the one worth keeping. It is read per
frame from the numbers the app printed, rather than by counting wheel notches,
because a gesture is smooth: the body absorbs about five notches and hands the
rest to the column by design.

## Next

1. **Decide what a zone island's close glyph means.** Either it works, with a
   way to reopen what it closed, or it is not drawn. `DESIGN.md` currently gives
   the side zone no close button, which points at not drawing it. This is a
   product answer and it is the owner's.
2. **Both drag-to-map gestures.** Still unverified, and blocked on data rather
   than code: the local mirror syncs `units 0`, so the Fleet picker has no rows
   to drag. Seed a unit catalogue and drive them.
3. **Give the top band a fill, or decide it does not need one.** `0:00:00 to 1x
   PAUSED` sits on the sea with no background of its own and the map's labels run
   through the hint text. Cheap to try, cheap to undo, and it is a design question
   rather than a defect.
4. **Deploy `aaee9bf`.** Not a code task. The client's default endpoint is the
   hosted dev API, which 404s on the readiness route, so the console cannot show
   a gate verdict against the environment it ships pointing at.

## State

Tree clean at `7213566`. `cargo test --lib`: 300 pass with the same two
pre-existing failures (`map_render::quad_bow_follows_the_compass` and
`sim::order_move_applies_then_higher_overrides_loudly`, both confirmed present
before this pass by stashing the diff and re-running). `cargo test --bin tfg`: 44
pass. The four new library tests are the scroll chain and its slack.

`AGENTS.md` used to say not to compile at all, which conflicted with
operating the app; it now carves out UI work explicitly, so the next agent does
not have to decide it per session.