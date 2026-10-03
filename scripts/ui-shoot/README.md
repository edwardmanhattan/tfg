# Photographing the console, and a Minos to photograph it against

Two things this directory exists for, both of which were harder to get working
than they look.

- `uimouse.py` — a pointer for a Wayland compositor, written because nothing
  installable would do. `ydotool` and `dotool` both need root; `/dev/uinput`
  carries an ACL for this user, so the device can be made directly, which is
  all either of them does anyway. Hyprland/libinput picks it up on creation, so
  nothing has to be configured compositor-side.
- `stack.sh` — the Minos stack with no root and no docker. Postgres and Valkey
  are not installed here and `docker.sock` is not accessible, so the Arch
  packages are fetched and extracted into a local prefix. Postgres then needs
  `LD_LIBRARY_PATH` for `libnuma` (from `numactl`, also extracted).

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