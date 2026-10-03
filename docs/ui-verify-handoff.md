# Handoff: verifying the console on a private display

Written at a clean pause, mid-session, with a working drive environment. A cold-start
agent should be able to pick this up and be photographing the console within one command.

## Intent

The UI overhaul is written and unit-tested, and none of it had been seen on screen. The
gap was closed with a virtual display so screenshots do not depend on what the operator
is doing, and then the render was driven through the states that matter. That found
defects which no test could see, at a rate of roughly one per interaction.

## What is verified, and how it was verified

Drove the real binary on Xvfb, captured with `import -window root`, and read the pixels.
Every claim below is from a render, not from reading code.

Fixed and confirmed on screen:

- **The no-session island's dead-end button.** It offered "Start a new session" and
  answered "name the game first", pointing at a create form that only renders once a
  session is HELD. Now carries the name field. `d4983ba`.
- **The gate is no longer derived client-side.** `execution_gate_blockers`,
  `plan_gate_blockers`, `NOT_READY`, `held_window`, `held_pace` and
  `TimeWindow::is_complete` deleted; `setup_checklist` is a formatter over the server's
  words. `d8df189`. Test count fell 295 to 293, which was the point.
- **The Planning column could not fit its own islands.** 1170pt declared into 624pt at
  the app's default window, and the declared height is a floor, so the shortfall could
  not be absorbed. Control and Fleet never rendered at all. Now scrolls. `b4d5b0d`.
- **A clip I broke while adding the scroll.** The island body scope re-clips to its own
  content rect, overwriting the band clip, so a scrolled island painted over the top
  band. `d674a47`.
- **Every arrow was a `.notdef` box.** epaint's default `Proportional` family omits
  Hack, which is the face that carries the arrows; its own `Monospace` family already
  lists Hack second annotated "fallback for √ etc". Appended one font NAME. `cb1505d`.
- **"Retry the sync" could not work.** A 404 means the server does not publish the
  route, so it now says so. `b52cfcc`.

Proven by driving, not yet a code change:

- The published gate works end to end. Against local Minos the checklist quotes the
  server's blockers verbatim: "the planned window is incomplete (both actual_end and
  assumed_end are required...)" and "no units assigned".
- Failing closed is real. Against the hosted API the checklist said "could not be read"
  rather than reporting zero blockers and lighting the Enter button. The old local
  derivation would have computed a clear gate from a roster of zero.

## Bring the environment up

Nothing is running between sessions. `/tmp/opencode` survives a reboot but not a clean,
so assume it is gone.

```bash
cd ~/Work/Crossnet/tfg

# Postgres 5433 and valkey 6380, unpacked into /tmp/opencode/stack, no root.
bash scripts/ui-shoot/stack.sh up

# Minos. The checkout is on `wip` at aaee9bf, which is the readiness commit —
# this is the only environment where GET /games/{id}/readiness resolves.
cd ~/Work/Crossnet/minos
go build -o /tmp/opencode/minos ./cmd/minos
go build -o /tmp/opencode/migrator ./cmd/migrator
# roles FIRST: they are cluster objects and a migration will not do it.
set -a; . /tmp/opencode/minos.env; set +a
export PATH=/tmp/opencode/stack/usr/bin:$PATH LD_LIBRARY_PATH=/tmp/opencode/stack/usr/lib
MINOS_APP_PASSWORD=local-app-pw MINOS_RO_PASSWORD=local-ro-pw \
  PGHOST=127.0.0.1 PGPORT=5433 PGUSER=minos PGPASSWORD="$PG_PASSWORD" PGDATABASE=minos \
  bash scripts/init_db_roles.sh
/tmp/opencode/migrator up --env /tmp/opencode/minos.env
/tmp/opencode/migrator --allow-superuser-seed seed-superuser --env /tmp/opencode/minos.env
setsid /tmp/opencode/run-minos.sh &      # 8099, API at /api/api/v1

# The private display.
bash scripts/ui-shoot/xvfb-up.sh         # :99, 1920x1200, no root, no compositor

# The app. WAYLAND_DISPLAY MUST be unset or winit opens the window on the
# operator's real desktop and :99 stays empty.
setsid env -u WAYLAND_DISPLAY DISPLAY=:99 \
  TFG_MINOS_HOST=http://127.0.0.1:8099/api/api/v1 \
  TFG_ZONE_DEBUG=1 LIBGL_ALWAYS_SOFTWARE=1 \
  ./target/debug/tfg > /tmp/opencode/tfg.log 2>&1 < /dev/null &
```

`/tmp/opencode/run-minos.sh` sources `minos.env` then execs the binary. Source it rather
than assembling the environment with xargs: `SUPERUSER_NAME="Super User"` has embedded
quotes and will break `env $(cat ...)`.

## Drive it

`scripts/ui-shoot/xdrv.c`, compiled by `xvfb-up.sh` to `/tmp/opencode/xdrv`. XTEST, so it
needs no compositor. Commands: `place X Y W H`, `click`, `rclick`, `mousemove`, `drag`,
`wheel N`, `type`, `key`, `focus`, `list`.

```bash
export XDRV_DISPLAY=:99 DISPLAY=:99
X=/tmp/opencode/xdrv
$X focus
$X key Tab; $X type "supersuser"
$X key Tab; $X type "tfg-dev-password"
$X key Tab; $X key space          # 12s, then the console is up
$X click 183 352; $X type "Kivu-7"; $X click 97 383   # create the session
$X mousemove 180 400; $X wheel 15                        # scroll the zone
import -window root /tmp/opencode/shots/x.png
```

**Log in with Tab, not clicks.** The login card is centred, so it moves when the error
line appears or disappears, and every remembered coordinate goes stale. Tab from a fresh
launch is stable. This cost several wasted rounds.

`TFG_ZONE_DEBUG=1` prints the zone's band, every island's position and height, the
overflow, the scroll offset, the hover point and the raw scroll delta. That line
settled three separate bugs and is the first thing to reach for.

## Gotchas that will cost you an hour otherwise

- **The window is hardcoded to 1040x640** (`src/main.rs`, `ViewportBuilder`). That is the
  size most defects reproduce at, and the size to test at first. `xdrv place` resizes it.
- **`WAYLAND_DISPLAY` leaking into the app's environment** puts the window on the real
  desktop while `:99` has no window at all, which looks exactly like a crash.
- **Never `pkill -f 'target/debug/tfg'`.** The pattern matches the driving shell's own
  command line and kills the shell. Use `pkill -x tfg`.
- **Text input must map printable ASCII by character code.** Latin-1 keysyms are the char
  code. `XStringToKeysym("-")` returns `NoSymbol` because it wants a keysym NAME, and
  silently dropping both hyphens from a password produces a field of sixteen dots and
  `unauthorized`.
- **Capture after the app has settled.** Tiles that have not finished loading photograph
  as beige blocks and stripes, which reads as a map corruption bug and is not one.
- **`island_on_band` culling is total.** An island outside the band does not paint wrong,
  it does not exist, and it cannot own input. Any "this control is missing" report should
  be checked against the zone geometry log before anything else.

## Next

1. **The three modals and the two map gestures.** Unopened. The Scenario composer button
   is in the Essentials island; the Fleet picker and Player picker are in theirs.
   Preparation is reachable now that the checklist is honest. This is the largest
   unverified surface and where the defect density has been highest.
2. **The prose truncation, mechanism unestablished.** A label loses the last two
   characters of its first line at the wrap boundary: "…without one is" renders as
   "…without one i", while line two is intact. Verified at 4x. NOT the band clip, which
   is ruled out because every intersection in the clip expression can only narrow it to
   the content rect. Either something here wraps against a rect wider than the clip, or
   egui measures the glyph run short and the last glyph overhangs. The cheap experiment
   is to widen the island by eight points and see whether the `s` returns; that
   separates the two. Do not patch this without finding out which it is.
3. **Deploy `aaee9bf`.** Not a code task. The client's default endpoint is the hosted dev
   API, which 404s on the readiness route, so the console cannot show a gate verdict
   against the environment it ships pointing at.

## State

Tree clean at `b52cfcc`. `--lib` 295 pass with two pre-existing failures
(`map_render::quad_bow_follows_the_compass` and
`sim::order_move_applies_then_higher_overrides_loudly`), `--bin tfg` 49 pass.

Longer reasoning lives in `docs/ui-overhaul-status.md`, and each decision with its
evidence in `docs/decisions/ui-overhaul.tsv`.

`AGENTS.md` says not to compile because the operator compiles it themselves. This session
built on their instruction to operate the app, which overrode it. That conflict is worth
resolving in the file rather than per session.