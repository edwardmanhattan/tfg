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

## The loop, in one paragraph

Write the test. Run it. Make it fail for the right reason. Fix the code.
Run it again. `cargo test --lib` — six seconds, and the four known
failures in `book`, `map_render`, `roster` and `sim` are known. Only then
shoot the states the change touches, read the taint line, and look at the
crop. Report the change with the capture's name and the test's name
attached, so the next person does not have to take the word for it.
