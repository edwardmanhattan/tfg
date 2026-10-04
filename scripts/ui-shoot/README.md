# Photographing the console, and a Minos to photograph it against

## Start here

```bash
bash scripts/ui-shoot/env-up.sh          # everything, idempotent
bash scripts/ui-shoot/drive-to-session.sh  # launch, sign in, pick the session
bash scripts/ui-shoot/verify-scroll.sh    # the scroll regression, 17 checks
```

`docs/ui-verify-handoff.md` has the rest, including the gotchas that each cost
an hour.

## What is here

- `env-up.sh` — the whole bring-up in one command. Postgres, valkey and Xvfb
  unpacked into local prefixes because none are installed and there is no root;
  Minos built and started; Xvfb on `:99`; a game seeded in `planning` with a
  scenario book and a Game Master seat.
- `drive-to-session.sh` — launches the app, waits for it to take input, signs in,
  picks the session, and **puts the session back into `planning` first**. The
  zone's contents are a function of the session state, so a run that inherits
  `Readiness` photographs a different console.
- `verify-scroll.sh` — the zone's scroll regression. Asserts, per frame, that one
  wheel gesture moved one thing.
- `find-buttons.py` — finds a control in a screenshot by its fill colour, so no
  click target is ever a remembered coordinate.
- `check-chain.py` — the scroll-invariant walk, split out so it is testable and
  so the `python3 -c` inside a shell script is not a hundred lines of quoting.
- `shot.sh` — capture with a coordinate grid burned in.
- `click-sweep.sh` — clicks a grid and stops at the first probe-pixel change.
- `xdrv.c` — the X11 driver. XTEST, so it needs no compositor.
- `stack.sh`, `xvfb-up.sh` — the pieces `env-up.sh` calls.
- `seedgame.py`, `seat.py` — the seeded game and its Game Master seat.

## Two dead ends, kept because they cost time

`uimouse.py` is a pointer for a Wayland compositor, written because nothing
installable would do: `ydotool` and `dotool` both need root, `/dev/uinput`
carries an ACL for this user, so the device can be made directly. It is retained
because it is the only thing here that drives the operator's real display, which
is exactly why it is not the default.

Three findings from that attempt, each of which fails silently:

1. **A relative pointer cannot be positioned.** libinput applies its
   acceleration curve, so a delta of 100 arrives as 200. Measured: asking for
   (100,100) produced a cursor at (200,200), and asking for (1200,700) produced
   the bottom-right corner of the screen, because acceleration made the delta
   enormous and it saturated. Nothing reports an error.
2. **Absolute axes do not take on this kernel.** `UI_ABS_SETUP` must be called
   BEFORE `UI_DEV_CREATE` (afterwards it is EINVAL, from C as well), and even
   then the cursor pins to the screen corner. Not chased further.
3. **Key events need every key bit advertised.** Registering only
   `BTN_LEFT`/`BTN_RIGHT` and emitting `KEY_A` means the device is a mouse that
   makes key noises. Registering all of them changed nothing.

The resolution was to stop driving the real display at all. `Xvfb` plus XTEST
gives a display that is never the operator's and needs no compositor, and it is
what every script here uses.

## The Minos stack, with no root and no docker

Postgres and Valkey are not installed on this machine and `docker.sock` is not
accessible, so `stack.sh` fetches the Arch packages and extracts them into a
local prefix. Postgres then needs `LD_LIBRARY_PATH` for `libnuma` (from
`numactl`, also extracted). Minos validates `REDIS_PASSWORD` as required, so a
valkey with no `requirepass` is not an option.

## Getting a pointer to work

On Xvfb, `xdrv.c` and XTEST. The Wayland path below is kept for reference, and
its four `EINVAL`s are what it took to get there.

Four things are wrong before it works, and each one fails as a bare `EINVAL`
with nothing to say which field was wrong:

## Getting a pointer to work

Four things are wrong before it works, and each one fails as a bare `EINVAL`
with nothing to say which field was wrong:

1. **`struct uinput_setup` is 92 bytes here**, not the 1120 the fuller
   definition implies. This kernel's header has no `ff_effects_count` and no
   absolute-axis arrays. Read the size out of a compiled probe
   (`sizeof(struct uinput_setup)`) rather than counting fields.
2. **The size field of `UI_DEV_SETUP` is the struct size.** A stale 132 made
   the request `0x40845503` where the kernel expects `0x405c5503`.
3. **Device parameters go through the `UI_DEV_SETUP` ioctl, not a `write()`.**
   The legacy write path is refused; the header's own comment on the ioctl says
   it was added for this.
4. **`UI_DEV_CREATE` is `_IO`,** so no direction bit and no size. Encoding it as
   `_IOW` gives `0x40005501` instead of `0x00005501`.

And one that is not a kernel problem at all:

5. **The device is RELATIVE.** "Click at 1375,562" moves it *there from
   wherever it was*, and a freshly created device has no position — so the
   click lands somewhere arbitrary and the symptom reads as "the app ignored
   my click". `move_abs` pins the pointer to the origin with one large negative
   delta and then moves relative to it, both in the same frame so the sweep is
   never shown. Absolute axes would be the obvious fix and are unavailable:
   `UI_ABS_SETUP` is refused here too.

## The trap: one device per command loses the pointer's identity

Each `uim.py` invocation creates and destroys its own device. That works for a
single move, but a sequence of commands is then a sequence of unrelated
pointers, and **focus does not necessarily follow**. In the session that
motivated this file, the second round of clicks and keystrokes went to a
different window entirely and typed a password into a chat window.

So: keep one device alive for a whole interaction. `Mouse` is importable for
exactly that reason — do not shell out per action.

## Getting a session to photograph

Minos wants a database and a cache, and the superuser has to be seeded as a
separate step that is only reachable once the schema exists:

    ./stack.sh up
    set -a && . envs/minos.env && set +a        # or your own env file
    go run ./cmd/migrator up
    go run ./cmd/migrator --allow-superuser-seed seed-superuser
    go run ./cmd/minos

Two things cost time and are worth knowing:

- **The seeder's flag goes BEFORE the subcommand** (`migrator
  --allow-superuser-seed seed-superuser`). After it, `go` passes it to the
  subcommand and the acknowledgement is silently not set.
- **Restart Minos after seeding.** Casbin loads its policy set once, at
  startup, so an instance running before the superuser exists keeps denying
  that account — and a `pkill -f cmd/minos` does not kill the child binary
  `go run` built, so the old instance survives and keeps the port.

Then `seedgame.py` and `seat.py` create a game in `planning` with scenarios and
steps, and seat the account as Game Master. The seat matters: **the scenario
book is gated on GAME ROLE permissions, not application ones.** `/games/
scenarios` update is granted to the Game Master role, and the middleware's
administrative path needs `/system/game-content` read, which a stock
Administrator does not hold. So authoring scenarios means being in the
exercise.

## Photographing

`grim` captures the COMPOSITED output, so a background window cannot be
captured however mapped it is. A freshly launched window is on top for a few
seconds; that is the window of opportunity, because raising a window afterwards
is not available on this compositor at all — `hyprctl dispatch` is Lua-only,
the Lua surface is reachable only through the `__lua` binds the user's config
routes everything through, and guessing `hl.dsp.*` names does not work.

Also:

- `hyprctl clients` reports **LOGICAL** rects on a 1.25-scaled monitor.
  Multiply before cropping or you lose ~200px on the right and read it as a
  layout overflow. This caused a long false chase once already.
- The app dies when the shell call that launched it ends. Use
  `setsid nohup … &`.
- Do not resize or close the user's windows to get a cleaner shot.
## Where the Wayland path stopped

The pointer moves the compositor's cursor — `hyprctl cursorpos` confirms it,
and the login card's button was located by scanning for its cyan fill to within a
pixel. But **no button press and no key press ever reached tfg**, by coordinates
or by Tab order, so the three modals were unrendered and it was replaced by the
Xvfb path.

Three findings, each of which fails silently:

1. **A relative pointer cannot be positioned.** libinput applies its
   acceleration curve, so a delta of 100 arrives as 200. Measured: asking for
   (100,100) produced a cursor at (200,200), and asking for (1200,700)
   produced the bottom-right corner of the screen, because acceleration made
   the delta enormous and it saturated. Nothing reports an error.
2. **Absolute axes do not take on this kernel.** `UI_ABS_SETUP` must be called
   BEFORE `UI_DEV_CREATE` (afterwards it is EINVAL, from C as well), and even
   then the cursor pins to the screen corner — which is what a device whose
   axes have a zero range does with any value. Not chased further.
3. **Key events need every key bit advertised.** libinput decides a device is
   a keyboard from the bits registered at creation. Registering only
   `BTN_LEFT`/`BTN_RIGHT` and emitting `KEY_A` means the device is a mouse
   that makes key noises. Registering all of them changed nothing here, so
   something further along — seat assignment for a hotplugged device, most
   likely — is not delivering the events to the client.

## The focus guard was necessary and was NOT sufficient

`drive.py` re-reads the focused window before every keystroke and aborted if it
was not tfg. It did its job — the run that typed a password into a chat window
had no guard, and the run after it never typed outside tfg — but it only proved
tfg was focused *at the moment of the check*. It did not prove the compositor
would deliver the event.

Which is why nothing here drives the operator's desktop now, and why
`drive-to-session.sh` names the display rather than reading `DISPLAY` out of the
environment. An earlier version read `${DISPLAY:-:99}`, and inside an agent shell
`DISPLAY` is `:0`: the app opened a window on the operator's screen and the
driver then found no egui window on `:99`. The failure is silent and the blast
radius is whatever the user happened to have open.
