# Verifying a change, fast

The console is verified against the running binary, because a window is not
a type. But verification is where the time goes, and most of that time is
spent answering questions badly: with a screenshot where a unit test would
do, with three builds where one would do, or with a pixel hunt for a
defect the change never touched.

This is the order that works, and the reasons it is this order.

## 0. The operator does the looking.

**The operator does the visual check.** They will open the build and look at
it. What the agent owes is the build, the numbers and the honesty about
them — not a picture of every state, read back into the context window and
argued about. A capture is still worth TAKING (it is the evidence the
operator looks at, and it is cheap), but reading it into the context and
reasoning about the pixels is the operator's half of the job, and doing it
for them is what made the loop slow.

So: build, `cargo test --lib`, capture the states the change touches, and
stop. Read a crop only when a number says something is wrong.


## 1. Unit test first. Always.

`cargo test --lib` is six seconds. A capture cycle — build, launch,
sixteen seconds of app, screenshot, read — is about two minutes.

Any question of the form "what is X at Y" is a unit-test question:
`view::zone_band_at`, `view::zone_slot_origin`, `view::zone_stack_height`,
`chained_wheel`, `owning_island` are all pure functions over a viewport
and a slot list. A capture answers them in two minutes; a test answers
them in six seconds and leaves a witness behind.

Reach for the capture when the question is about **pixels** — a rule, an
edge, a clip, a colour, an alignment. Not before.

## 2. One probe, one build, one line.

A throwaway `eprintln!` is a fine tool, but a probe costs a build, and a
build costs minutes. So:

- Put every value you might want into **one** `PROBE` line, printed once
  per frame or once per event, rather than one probe per question.
- Take the values from the same place the render takes them. A probe that
  re-derives the geometry is a second implementation of it, and it will
  disagree with the first one and cost you a whole debugging pass.

## 3. One sweep, not six.

`scripts/gpui-sweep.sh <tag> "name:seed" ...` captures a set of states
against the hosted API. Name the states **the change touches** — a column
change is visible in planning, preparation and closure; an execution
change is not visible in any of them. Shooting the same three states
against three successive binaries is the most common way to spend an
afternoon here.

## 4. A capture is a witness, not an oracle.

The harness reports what reached the console while the picture was being
taken:

```
input during capture: 230 wheel, 0 click
TAINTED: the operator touched the console while this capture ran.
pointer: (1109,549) — OVER the app window, so hover states are live
```

**Read that line before you believe the picture.** A capture that caught a
live wheel is a photograph of a state nobody asked for, and it is
indistinguishable from a layout fault until you know. This is not
hypothetical: on 2026-10-20 a sweep's captures came back with the column
scrolled, and the hunt went looking for a layout bug that was not there.

The pointer line matters too, because hover states and the owned edge
follow it, and a hover state is not a gesture — the console cannot report
it, only the compositor can.

## 5. Do not hunt a pixel your change did not touch.

If the question is about something the diff does not touch, the answer is
already in the tree: look at the before-captures under
`.impeccable/review/*/before/`, or `git stash` and shoot the same state.
Fifteen tool calls of pixel forensics on 2026-10-20 established that the
island's cyan "owned" edge does not render — and that it never did, so it
was not the change's business.

## 6. Assert on numbers, not on pictures.

"Is there exactly one rule between these two fields?" is a one-pixel
column scan over the capture, not a 960×1080 image in the context window.
`magick shot.png -crop 1x900+399+40 +repage txt:-` and count the
colours is a fact; looking at the frame is an opinion.

`scripts/shot-assert.sh` does that, and `scripts/gpui-sweep.sh` runs the
assertions after each capture, so a sweep exits non-zero on a wrong
pixel:

```bash
scripts/gpui-sweep.sh after \
  "plan:TFG_GAME=9|at:0,300=cut-grey;runs:250=cut-grey=3" \
  "prep:TFG_GAME=41|at:0,300=cut-grey"
```

The palette is read out of `src/view/mod.rs` at run time, so an ink that
moves cannot leave the assertions quietly asserting about the old colour.
The primitives are `at:X,Y=INK`, `not:X,Y=INK`, `runs:X=INK=N` (contiguous
groups in a column — the right way to count rules, because a 1pt rule on
a 1.25x screen is 1.25 physical pixels), `rows:X=INK>=N`, `row:Y=INK>=N`
and `cyan:X>=N` (hue-based, so a partial blend still reads as cyan).

Two things to know about the numbers. The window's placement moves between
launches, so a capture's crop can sit a pixel off the window's own grid —
assert a COUNT (`runs:`, `cyan:`) where a position would be fragile, and
an exact position only where you have measured it. And a capture whose
log says the console fell back to its fixture roster is a picture of the
FIXTURE, not of the deployment: the harness says so, and so should you.

Full-frame `read` of a capture is the most expensive tool call in the
loop, and it is the one that costs the most context. Crop first, read the
crop, and read the full frame only when the crop is ambiguous.

## 7. The GPUI facts are written down.

The scroll hit test, the two hover predicates, the bubble order, and why
`stop_propagation` is the only thing that can stop a background handler:
`docs/gpui-gotchas.md`. Read it before reasoning about who receives a
gesture. It is eight tool calls of source-reading, done once.

## 8. The control plane has a real-binary smoke.

`bash scripts/control-smoke.sh` launches the console with the control server,
sends `tfgctl camera left` and `camera zoom-in`, and asserts on the numbers: the
centre moved west, the zoom rose by one, and a watching client received the
attributed `camera.changed`. It is the only test that proves a remote command
reached the shell, because the hub is written from the camera the shell renders.

It is a window launch, so it follows the capture rules: it kills only this
checkout's stale console, it needs both binaries built, and it leaves its
artifacts in `/tmp/opencode/control-smoke`. A failure prints the check and the
numbers it saw; the unit tests around the same code are
`cargo test --lib --offline control::` and they do not open a window.

Twelve checks since pairing landed: the last four pair a device against the
console, read the camera with the device's own token, revoke it, and assert the
table exists at `TFG_CONTROL_DEVICES` with mode `0600` and no plaintext token.
The device table is a throwaway path, so the smoke never leaves a paired phone
behind on the machine that ran it.

## 9. The headless host has its own smoke, and it needs no display.

`bash scripts/control-server-smoke.sh` is the sibling of the one above, and the
opposite shape on purpose. It starts `tfg-server`, drives it with `tfgctl`,
pairs a device, revokes it, restarts the process, and asserts 28 things —
including that a revoked token is still refused after the restart and that the
devices file is `0600` with no plaintext token in it. Nothing opens a window, so
it runs over ssh and in a container with no `DISPLAY`.

What it cannot prove: that anything was drawn. `tfg-server` owns a camera and
nothing else, so this is the test for *state agreement*, not for a picture. A UI
surface still needs §3.

Its artifacts land in `/tmp/opencode/control-server-smoke`, and the server log
there is the readable evidence: one line per applied command, with the revision,
the resulting camera and the client that asked.

Both smokes are the same bargain: a script that would fail loudly rather than a
screenshot that looks right. The pairing smoke earned its keep on the first run
by failing three checks on a wrong assumption about the CLI's JSON shape, which
is exactly the kind of thing that reads fine and is wrong.

## 10. A QR code is a picture. Decode it.

A QR matrix that is subtly wrong still *looks* like a QR code, so "it renders"
is not evidence. Two tools settle it:

- `qrencode -t ASCII -l M -m 0 -8 -v N` is an independent encoder. Compare
  module for module; every module must agree. One clean way to look at a
  disagreement is to dump all eight mask candidates from `qr.rs` (the ignored
  `qr::tests::dump_for_comparison` writes them) and check that qrencode's matrix
  equals one of them. If it does, the data, parity, placement and format bits
  are right and only the mask heuristic differs — both codes are valid.
- `zbarimg --quiet --raw` is an independent decoder. Write a PBM (`P1`) from
  the matrix with a four-module quiet zone, scale it up six times, and assert
  the decoded bytes equal the payload. Do the same for the code as the terminal
  actually printed it, by turning `█▀▄ ` cells back into two module rows: that
  tests the renderer, not just the encoder. Use a payload filled to exact
  capacity for every version; a wrong block table hides until a boundary.

The `tfgctl pair` path only draws when stdout is a terminal, so drive it through
a PTY (a Python `pty.openpty()` driver is enough) and pair the device over
`POST /v1/pair` while it waits. Redirect the same command and assert that no
glyph appears, so a piped script's output stays parseable.

## 11. A client follows a camera. Read the host's log, then the window's chrome.

A console in client mode (`TFG_CONTROL_SERVER`) has two observables that do not
depend on reading a map tile:

- On its first snapshot it posts `view.report` under its own
  `console-<pid>-<n>` client id, and the host's log names it. That proves the
  launch wiring, the WebSocket, the snapshot, and the shell's measured size all
  arrived.
- A remote `camera.set` must change the window. The zoom badge in the top bar is
  the cheapest numeric witness: two captures across a `tfgctl camera set` went
  from `11` to `6`, and about 18,700 pixels differed across a 768×838 window.

Do not assert on the host's camera for this. The host is where the camera came
from, so its value says nothing about whether the display followed. And do not
expect the console to post the remote camera back: it must not, and that silence
is part of the contract. The unit tests cover the channel
(`src/control/client.rs`), the host log covers the connection, and the capture
covers the shell.

## The loop, in one paragraph

Write the test. Run it. Make it fail for the right reason. Fix the code.
Run it again. `cargo test --lib` — six seconds, and the two known
failures in `map_render` and `sim` are known. (The `book` and `roster`
ones were fixed with the local-first planning slice: the book test
contradicted `BookDraft::prune`'s own rule, and the roster's write
order was the bug the test was always asserting against.) Only then
shoot the states the change touches, read the taint line, and look at the
crop. Report the change with the capture's name and the test's name
attached, so the next person does not have to take the word for it.
