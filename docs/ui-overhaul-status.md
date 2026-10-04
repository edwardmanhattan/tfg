# UI overhaul — status and continuation

An overhaul of the command console's information architecture, in progress.
This file is the map: what is decided, what is built, what is not, and what
the next person has to know before they touch anything.

Read `DESIGN.md` for the visual system and this file for the work.

## Why this is a continuation, not a finish

The design system is rewritten, the architecture is decided, and the **side
zone is running**. The app boots to the new column over the map: the Operator
island, then the islands for whatever `GameState` says.

What is *not* done is the rest of the frame and all three modals. The top
zone still has the mode toggle on it, the Settings modal does not exist, and
the Fleet and Player pickers are still the old inline widgets rather than
modals. So the console is half-overhauled: the largest surface is new and the
smaller ones are pending.

The reason the zone went first is that it is the surface everything else
hangs off, and because `src/tokens.rs`, `src/chrome.rs` and `src/gamestate.rs`
hold no tfg types and therefore compile in `proto/p5-epaint` without building
the MapLibre core.

## Landed, and verified

| What | Where | How it was checked |
| --- | --- | --- |
| Palette and metrics as one module | `src/tokens.rs` | 4 unit tests; replaces ~35 hex literals at their call sites |
| `apply_ops_theme` reads from it | `src/main.rs` | last duplicate gone |
| ADR-0016's rim, neutral at rest and lit on input | `src/chrome.rs` | rendered on the real shipped file, both states |
| Ownership resolved in one place | `chrome::owning_island`, `chrome::island_owned` | a sweep over the column asserting no point is claimed twice |
| Band texture, confined to the band | `chrome::paint_band_texture` | rendered; the first pass overflowed onto the body |
| Zone geometry: dock, stack, origin | `chrome::zone_island_origins`, `zone_origin` | 4 tests pinning the origin exactly |
| `GameState` as one axis | `src/gamestate.rs` | 6 tests, including that `"dynamic"` is not a wire value |
| The zone itself, state-driven | `ShipApp::side_zone` | **ran the app and looked at it** |
| Camera correction | `ShipApp::visible_center` | 2 tests in `map_render.rs`; the first version was wrong and the test caught it |
| Trailing note in the title band | `Island::with_trailing` | rendered; the first pass collided with the close button |
| The Operator island is a real identity | `src/backend/auth.rs` `parse_user` | 3 tests plus a run: the role reads `Administrator`, not `APP ROLE 1` |
| The top zone, one row, floating | `ShipApp::top_zone` | run at 960 points: link, headcount, hint, zoom, clock, Settings, sign-out |
| Settings modal owns the mode and the layout preferences | `ShipApp::settings_modal` | the mode toggle is out of the band |
| Link state as a type, not a parsed sentence | `LinkState` | five variants, each with its own status ink |
| "In room" is a fact | `Participant::joined_at` | the server already returned it; the parser threw it away |
| Reduced motion is wired | `ui()` | one `motion_secs` constant, 0 when reduced |
| The modal primitive, with a real input-swallowing backdrop | `src/chrome.rs` `modal`, `Modal` | 4 geometry tests; the negative-body test failed twice before it passed |
| The scenario book's client API | `backend::master` `GameScenario`, `GameScenarioStep`, 8 methods | 6 tests: the `HHMM` rules, nested-step parse, no-id refusal |
| The Scenario Composer modal | `ShipApp::composer_modal`, `composer_body` | `cargo check` clean; **not looked at** (no compositor on this machine) |
| The composer draft's rules | `ComposerDraft::problem` | 5 tests: half-window, malformed, backwards, no-window, empty content |
| The Fleet Picker modal | `ShipApp::fleet_picker_modal` | `cargo check` clean; **not looked at** (no compositor) |
| A clear backdrop for drag-to-map | `chrome::Backdrop` | 2 tests: it drops the dim and nothing else, and a dim is actually dimmed |
| A release over a panel is not a drop | `main::drop_lands_on_map` | 3 tests: the panel edge half-open, the empty case, and TWO panels where one would pass both |
| The Player Picker modal | `ShipApp::player_picker_modal` | `cargo check` clean; **not looked at** (no compositor) |
| The seat note's rule | `main::role_may_need_command` | a test that says exactly what the schema can support and no more |
| Island roster is read-only, editor is in the modal | `users_roster_ui` / `roster_editor_ui` | a source-level check on the Id collision, which no screenshot would reveal |

`cargo test --lib`: 286 pass. `cargo test --bin tfg`: 43 pass. Harness: 91 pass. Two failures are pre-existing and unrelated
(`quad_bow_follows_the_compass`, `order_move_applies_then_higher_overrides_loudly`);
both were confirmed failing on a stashed tree before this work.

## Decided, and not up for re-litigation

These were settled against the Minos backend on `wip` (the source of truth)
or settled by rendering. Reopening them means re-reading the evidence column
in `docs/decisions/ui-overhaul.tsv`.

| Question | Answer | Where |
| --- | --- | --- |
| Zones | Three: top band, side column, modal. Nothing else. | `DESIGN.md` "The three zones" |
| Side zone geometry | Floats **over** the map, never beside it. Docks left or right, remembered. | `DESIGN.md` "Side zone" |
| Side zone contents | A function of one `GameState` value. Never empty. | `DESIGN.md` "The five game states" |
| Mode toggle | Out of the toolbar, into a settings modal. Simulation does not announce itself. | top zone section |
| Aesthetics | *Tactical with heat*: one cyan accent, lit rim, texture on the band only. | `DESIGN.md`, ADR-0016 |
| The rim | Cut Grey at rest, Radar Cyan only on the island owning input. | ADR-0016 |
| Vent slots, body scanlines | Rejected on rendered evidence. Do not reintroduce. | `docs/decisions/ui-overhaul.tsv` |
| Island open/close motion | Reinstated, 150–200ms `cubic_out`. Supersedes the old ban. | `DESIGN.md` Motion |
| `dynamic` | Not a wire value. Bind labels to `maneuver`/`static`/`scenario`. | decisions TSV |
| The scenario book | Per-game, one book, no global catalog, personnel cannot read it. | decisions TSV |

## Backend facts the UI now depends on

Read from `Crossnet/minos` on `wip`. These were previously assumed and were
wrong in several places.

- **`/users/me` is enough for the user island.** `photo_url` (nullable,
  presigned), `name`, `email`, and app roles with ids *and* names. The
  current client parses only `id` and `app_role_ids`
  (`src/backend/auth.rs:200-208`); the rest is unread. Default person glyph
  when `photo_url` is null, and the backend cannot set a photo yet.
- **A game's roles are per-game clones**, not the templates. A game's
  `Commando` id is not the template's id (`000048_per_game_roles`). Use
  `GET /games/{id}/roles`, never `GET /game-roles`, to fill a seat.
- **No role flag says "this role needs a fleet".** The only role flag is
  `is_judge_side`, and it is the *inverse*: a judge-side participant may not
  hold a hull. The real constraint is one person commands one thing
  (`game_service.go:2756-2790`), enforced at assignment time.
- **`GamePace` (`standard`/`fast`) is unmodelled by the client.** `fast`
  waives exactly one readiness gate. Worth surfacing in Planning.
- **A field the parser drops is a field the island cannot show.** This bit
  TWICE: `parse_user` threw away four of five `/users/me` fields, and
  `parse_game_detail` threw away six of the game's. Both were write-only in
  the client — the create/edit form sent them and nothing read them back. The
  habit that catches it is keeping the whole response, not cherry-picking:
  `ShipApp::held_detail` now stores `GameDetail` whole, so the next unparsed
  field is a visible omission rather than a silent one.
- **Two absences are the same shape on the wire.** Minos omits `area` both
  when it is unset and when it is withheld from a participant, so the client
  cannot tell them apart from the response. `GameDetail::area_is_withheld`
  answers it from WHO IS ASKING instead — a Game Master is never withheld.
  Reading the game state alone would tell the person editing the plan that
  their own area is hidden.
- **There is still no endpoint to set a user photo.** `photo_url` is only
  ever populated by a bootstrap seed or by a presign of an existing key, so
  most accounts keep the default glyph. That is a backend gap, not a client
  one, and the glyph is the correct thing to ship until it closes.
- **The scenario book is nested.** `GET /games/{id}/scenarios/{sid}` returns
  the scenario *with* its steps; there is no step-list route, and no
  endpoint returns the current step to anyone.

## The defect that mattered most, and what is left of it

Found during this overhaul, and it is the largest thing in the log.

**The console could not start an exercise. Any exercise.** `GameReadiness.CanExecute`
(`internal/models/game_participant.go:179`) needs a planned window —
`WindowSet` is `actual_end IS NOT NULL AND assumed_end IS NOT NULL`, read in
the same statement as the other four values, with nothing defaulting either
end. The only writers are `POST /games` and `PATCH /games/{id}`. The client
sent neither — and in fact had **no write path for any of the four window
fields**, start or end. `GameUpdate` was six `Option<String>` text fields.

So the button behaviour was worse than a dead one. `zone_ready_body` gates on
`clear = blockers.is_empty()`, and the window was not among the blockers. Once
the four conditions the client *did* know about were satisfied, the button
**enabled**, the operator clicked, and the server refused with a 409 naming a
field the console had no control for. The checklist reported "0 outstanding"
and was confidently wrong.

That makes three client re-derivations of one server rule now known to drift,
and the honest reading is structural rather than per-field: pace was the
fourth. The client's copy has never agreed with the server's on anything more
than the two counts the server hands it.

Now: window and pace are authorable on the edit form, and the gate is ONE
function (`execution_gate_blockers`) that `setup_checklist` formats rather
than reimplements.

**DONE, in both repos.** Minos now publishes the gate:
`GET /games/{id}/readiness` (commit `aaee9bf` on `wip`) answers
`{can_execute, blockers, fast}`, where `blockers` is the SAME list a refused
transition names, split into sentences. It is a GET on the path the two
readiness *writes* already use, and it widens nothing: every value was already
readable through a route this client has.

tfg consumes it and the local derivation is **gone** —
`execution_gate_blockers`, `plan_gate_blockers`, `NOT_READY`, `held_window`,
`held_pace` and `TimeWindow::is_complete` all deleted. `setup_checklist` is a
formatter over the server's words now; the only thing it adds is the
participant names behind the readiness count.

Two consequences worth knowing:

- **`roster_gap` no longer affects the gate at all.** The client used to fall
  back to "did *you* declare" when it could not read the roster, which told a
  caller the gate was clear when the server disagreed. The server counts
  regardless of what the caller may read.
- **An unknown gate refuses rather than reading as clear.** Before the first
  bundle lands, and after a failed readiness read, the checklist says so —
  never an empty list, because empty means "nothing is outstanding" and that is
  the one answer that must never be invented.

Test count went DOWN (293 from 295) and that is the point: tests for a rule
that no longer exists were deleted with it.

## Not built yet, in the order it should be

Ordered so each unit is verifiable on its own and each one shrinks the risk of
the next.

1. **The three modals and both drag-to-map gestures are still unrendered.**
   A pointer now exists and a real session exists, but **no button press and no
   key press has reached the app** — see `scripts/ui-shoot/README.md` for the
   three silent failures behind that. Do not drive input on this machine's
   desktop until event delivery is confirmed: the failure is silent and the
   blast radius is whatever window the user has open.
   Both surface blockers were removed and the work got as far as the login
   card:
   `scripts/ui-shoot/` has a working uinput pointer (no root needed) and a
   Minos stack with no root and no docker, and a real session was created with
   a Game Master seat, two scenarios and three steps. It stopped because each
   `uim.py` call creates its own pointer device, and focus did not survive the
   sequence — the second round of keystrokes went to a different window. Keep
   ONE device alive for a whole interaction, as the README says. What is still
   unverified is the part a session exercises: whether `unit_picker_ui` fits a
   980x620 body, and whether a drag really leaves the modal and lands on the
   sea.
2. ~~Island heights are hand-guessed constants.~~ DONE — the column now
   measures itself (see below). What remains is the CEILING: an island whose
   content is genuinely unbounded is clipped at the window rather than
   scrolled. That is a deliberate choice, and unlike the rest of this it is
   not a tested one.

## Traps worth writing down

- **`egui::Panel` is the wrong tool for the side zone.** It shrinks the
  `CentralPanel`, which is exactly what the design forbids. The zone is an
  `Area` in the map's `Background`-layer world, painted above it.
- **`unproject_mercator` takes a viewport-relative pixel, not an offset.**
  `visible_center` got this wrong on the first run and the test caught it.
  The trap is that the function name reads like it takes a delta, and every
  other camera helper here is passed offsets.
- **`chrome.rs`, `tokens.rs` and `gamestate.rs` have no tfg types on
  purpose.** They are compiled standalone into `proto/p5-epaint` by
  `#[path]`, which is the only way they get verified without building the
  MapLibre core. A `use crate::store::…` in any of them silently costs the
  harness, and the harness is what makes the visual system falsifiable.
- **Legacy islands still park at x=8.** `zone_owns_left_edge` suppresses
  Login, Log, Messages and Roster while the zone holds that edge. Presentation
  still has no zone and still needs them, so they cannot just be deleted.
  When the zone docks RIGHT, that gate stops suppressing anything and the
  conflict moves to the other edge — that case is untested.
- **The mode card is still a startup gate.** `Onboard::Mode` asks before the
  shell appears, which is the last place Simulation announces itself. The
  toggle now lives in Settings as well, so the card and the modal are the
  same decision asked twice. Collapsing the card is the last piece of the
  owner's item 1 and it is the reason `Onboard` will lose a state.
- **`chrome::modal` is verified by tests, not by a render.** The harness has a
  `--modal` path that draws the real modal over the real zone, but the
  capture did not complete, so the dim strength and the form-over-map
  contrast are unlooked-at. Run `--island --modal` before trusting either.
- **Screenshot arithmetic lies.** hyprctl reports window `at`/`size` in
  LOGICAL pixels and the crop has to be scaled by the monitor scale. A
  capture that skipped that cropped ~200px off the right of every image,
  which reads as a layout overflow. It cost a real chase of a bug that did
  not exist. The screenshot scripts live outside the repo and are the
  harness for this; they are worth keeping alive and not worth trusting
  without checking the rect.
- **Island positions are app-owned `Pos2` fields** and nothing persists
  across launches. The zone owns layout now, but an island can still be
  dragged out of the column, which means "the zone owns the layout" and "the
  operator may move an island" are currently both true and in tension.

## Verified on the real app, and what it cost

The compositor came back mid-session, so part of this was checked on the
running console against a real tile map rather than in the harness. It found
a bug no geometry test could have.

**The Operator island overflowed.** It was 112pt; its content needed more;
and the status line painted OUTSIDE the panel, over the map, half-occluded
under the next island's title band. In the screenshot it read as a label
belonging to the island *below*. Every rect involved was correct — which is
exactly why the four zone-geometry tests passed and this still shipped.

Two fixes, and the order matters:

- `island_owned` now **clips its body**. A `max_rect` only tells the layout
  where to stop; it does not stop the painter. This generalises to all seven
  islands including the four new Execution ones, and turns "corrupts the
  neighbour" into "a mistake". It is a mitigation, not a fit — see the
  remaining-work note.
- The status line came off the identity island (it was a transient connection
  fact, which the top zone's link state already reports) and the island is
  now 140pt, measured by rendering. 104 and 128 both clipped the button.

**The dim was judged against the wrong background, and that mattered.** The
harness has no background of its own — the window is transparent, so every
capture showed the near-black desktop wallpaper. Judging a *black* overlay
against a *black* background tells you nothing, and the real tile map is
bright. `p5-epaint --bright` now paints a map-like ground (pale sea,
near-white land, coastlines, grid, place labels) for exactly this. On it the
two candidates are nowhere near each other:

| alpha | land under the dim | what survives |
| --- | --- | --- |
| 150 | srgb(100,101,96) | almost nothing; land and sea collapse to one value, coastlines vanish, place labels all but disappear |
| 104 | srgb(143,143,136) | land/sea/grid/coastline all hold, labels stay legible, islands behind still recede |

Getting there also cost a bug: the first version of `--bright` painted
*inside* the `island_mode` branch, after the modal's early `return`, so it
never ran. A render that shows exactly the previous background is the tell.

**The column now measures its own islands.** The overflow detector was
already reporting "content needs more"; the fix was to USE that. Each island
draws at whatever height it was given and publishes what it would have
preferred, and the next frame stacks with that. One frame of lag, no
double-run — which is the whole trick, because the alternative is running
every body twice per frame and every write a body performs would fire twice.

Verified live: with the Operator island deliberately set to 70pt it reported
the overflow **once** and then self-corrected. Before this, 70pt would have
overflowed on every frame forever.

The measurement rides the context's TEMP storage rather than a `HashMap` on
the app — temp storage is already scoped to one frame, so a parallel field
would be a second source of truth that can disagree with it. The clamp (never
below the declared constant, never past the window, and a `NaN` measurement
refused outright, because `NaN` through `clamp` silently poisons a rect and
drops every island under it) is a pure function with five tests.

**A silent clip is worse than a loud one.** Clipping the island bodies fixed
the corruption but turned "corrupts the island below" into "a button quietly
vanishes", which reads as a bug in the form. So `island_owned` publishes
whether its body fit and the zone logs it. Verified in both directions: with
the Operator island at 140pt there are zero reports; deliberately shrinking
it to 90pt produces exactly one.

The threshold is one text line, and it is a measurement rather than taste. At
0.5pt every island reported, because a layout's `min_rect` runs ~4pt past
the last control for trailing spacing — the Operator island measures 98.4
against 94 available and renders with visible slack. That false positive
briefly looked like a second bug in the "New session" island, which was worth
chasing down rather than shipping. The real 112pt overflow ran ~52pt over.

**Also confirmed by render:** the mode card is gone and the app goes straight
from login into the console (the keyring token still logs in, so this was a
real end-to-end pass); the top zone is one row over a live map; the zone is
docked left with the chamfer and the lit-rim rule reading correctly against
a bright background; and `MODAL_BACKDROP_ALPHA` at 150 erased the map, so it
is now 104.

**How to photograph the console** — the non-obvious parts:

- The app dies when the shell call that launched it ends, so launch with
  `setsid nohup ./target/debug/tfg > log 2>&1 < /dev/null &`.
- `grim` captures the COMPOSITED output, so a background window is not
  capturable however mapped it is. A freshly launched window is on top for
  about fifteen seconds, which is the window of opportunity. After that
  whatever the user focuses covers it, and there is no way to raise it —
  `hyprctl dispatch` is Lua-only and rejects every focus form, including
  `address:0x…`.
- The monitor is 1920x1080 at scale 1.25. `hyprctl clients` reports LOGICAL
  rects, so multiply by 1.25 before cropping, or you lose ~200px and read it
  as a layout overflow. That caused a long false chase once already.
- Do not resize or close the user's windows to get a cleaner shot.

## Verification

`proto/p5-epaint` is the harness and it now carries the heat passes:

```
cargo run --manifest-path proto/p5-epaint/Cargo.toml -- --variant 0 --heat 1
cargo run --manifest-path proto/p5-epaint/Cargo.toml -- --variant 0 --heat 3
```

`--heat N` selects a pass, `--heat-dark` renders over an empty stage instead
of a map, `1`/`2`/`3` and `7`/`8`/`9` switch silhouette and heat from the
keyboard, and `D` flips the stage. A screenshot script that verifies its own
capture lives outside the repo; it polls the compositor for the window, crops
using the reported window rect, and probes a panel-body pixel, because a fixed
sleep twice produced a plausible PNG of the wrong window.

Look at the renders. Two of the four heat passes were rejected by looking,
and both failures were invisible in the code.
## What the render actually shows (private display, first pass)

Driven on Xvfb with `scripts/ui-shoot/xdrv.c`, not on the real screen. Four
findings, three of them defects I introduced or inherited, and one I am not yet
able to attribute.

**1. The readiness endpoint 404s against the deployed backend.** The route
exists locally at `minos internal/routers/game.go:477`, and `aaee9bf` is the
commit that added it, but the API the console actually points at answers
`HTTP 404: Not Found` for `/games/{id}/readiness` while `/games/{id}` and
`/games/{id}/participants` answer. So the client is ahead of the server it
depends on. This is not a hypothetical: the console cannot show a gate verdict
against this environment until that commit is deployed.

What is worth keeping is how it failed. The checklist read

    • the readiness check could not be read — retry the sync

rather than reporting zero blockers, and `unknown_gate_blockers` is what put
that sentence there. A client that had kept the old local derivation would have
happily reported "0 outstanding" and lit the Enter button, because a roster of
zero and a window nobody declared still add up to a clear gate. Deleting the
derivation did not just remove a second opinion; it removed the only thing that
could have answered wrongly.

**2. FIXED. The Planning column could not fit its own islands, and the ones
that did not fit were not merely clipped but GONE.** Planning declares four islands at 380 + 150 + 320 + 320 = 1170pt. The
side zone at the app's hardcoded `1040x640` has `640 - ZONE_ISLAND_GAP` = 624pt.
The declared height is a floor (`fitted_island_height` clamps to
`declared.min(ceiling)`), so the shortfall cannot be absorbed: Essentials is
clipped mid-sentence and **Control and Fleet never render at all**. The clock
multiplier and the fleet are unreachable in the state where you set the clock and
assemble the fleet.

Every rect involved is correct, which is why no geometry test caught it. The
loud clip reports only Essentials, because Essentials is the one whose *body*
overflowed its box; Control and Fleet did not overflow, they were never given
room. A clip report is evidence that a box is too small, not evidence that the
column adds up.

**3. FIXED. Two codepoints rendered as `.notdef` boxes.** `→` (U+2192, 44 call sites)
and `✓` (U+2713, 7 call sites) both come out as hollow rectangles, so every
"Enter preparation →" and every "· ready ✓" is a button with a broken glyph in
it. No `FontDefinitions` is registered anywhere in `src/`, so this is egui's
bundled fallback failing to cover two characters the UI leans on heavily. Font
coverage is resolved on the CPU into the atlas, so this is not an artefact of
software rendering. It is invisible in code review and obvious in a screenshot.

**4. Unattributed: the map corrupted after a window resize.** At 1600x1100,
resized from 1040x640, the map rendered large beige blocks, vertical stripe
artefacts and a black band across the bottom. At 1040x640 it was correct. That
is either a viewport that does not re-fit on resize, which would be a real bug,
or an llvmpipe artefact, which would not be. One more capture at a second size
settles it and I have not done that yet, so it stays a suspicion.

### The fix, and the three bugs inside it

The column now scrolls. Three things had to be true at once, and each was
wrong on its own first:

**The island's clamp fought the scroll.** `island_owned` clamped an island's
position into the viewport so a title drag could not carry it off the window.
In a scrollable column a position below the window is *correct* — that is what
a column taller than its band IS — so the clamp pulled Fleet from y=1026 up to
y=320 and the scroll then carried it off the top. The clamp now applies only
when a drag actually happened. A computed position is trusted; a dragged one is
constrained.

**The sign was inverted.** egui reports a POSITIVE Y for content moving DOWN,
so scrolling down the column arrives as a negative delta. `+=` drove the offset
below zero where the clamp pinned it, and the symptom was a wheel that did
nothing at all — with the scroll value sitting at exactly 0, which looks like
the input never arrived rather than like arithmetic that went the wrong way.
Instrumenting the delta settled it in one run: `delta=-96.8`.

**Scroll is a view transform, not a position.** It is applied at draw time
inside `island_owned`, so the stored position keeps meaning "where this island
lives in the column" and the drag clamp stays a statement about dragging.

Verified by driving: at scroll 0 the band shows Operator and the top of
Essentials; at scroll 560 it shows Essentials' tail, **Control** (game time
multiplier, with its rim lit, so ADR-0016 ownership still holds under scroll)
and **Fleet**; at scroll 1200 it shows Players, whose checklist is now quoting
the server's blockers verbatim — "the planned window is incomplete (both
actual_end and assumed_end are required...)" and "no units assigned" — which is
the published gate working end to end against a Minos that has `aaee9bf`
deployed.

**The original section 2 below is kept as written, because the arithmetic is
what the fix is accountable to.**

### Why every arrow was a box

Not a font that was missing. epaint's default `Proportional` family is
`[Ubuntu-Light, NotoEmoji, emoji-icon]` — with **Hack omitted** — and Hack is
the face that carries the arrows. Reading both cmaps: Ubuntu-Light covers none
of U+2192, U+2190, U+25B8, U+25CF or U+221A; Hack covers all five. epaint's own
`Monospace` family already lists Hack second, annotated *"fallback for √ etc"*,
which is this exact fix applied to the wrong family.

So all 44 arrow labels were boxes. The fix appends one NAME to the Proportional
list — the bytes are already in `font_data` under `"Hack"` — which means no
vendored font, no new dependency, and Latin still comes from Ubuntu-Light because
it stays first.

No test can see this class of bug. The string in the source is correct and the
glyph lookup *succeeds*, returning the missing-glyph box; nothing about the
program is wrong except what it draws.

The ticks are a separate problem with a separate answer: U+2713, U+2714 and
U+2611 are in **no** bundled font, so no fallback can rescue them. Five of the
seven sat on text that already said the fact (`ready`, `assigned`, `placed`), so
the glyph was decoration and is gone. The two that actually carried state, the
`to` and `cc` toggles, became `●`, which Hack does cover.

### "Retry the sync" was advice that could not work

The gate fails closed, which is right, but it failed with a sentence that was
wrong in the one case that mattered. Against the deployed dev API the checklist
read *"the readiness check could not be read — retry the sync"* — and the sync
was never going to succeed, because the server does not publish the route. That
is a client ahead of its server, not a connection having a bad moment, and the
two want different sentences.

`BackendError::Api` already carries the status, so this is classified rather
than guessed: `NotRead` (nothing has run yet), `Unread` (failed, may work next
time) and `Absent` (404, this server has no such route). Only `Unread` keeps the
word "retry". `Absent` says the server does not publish the check.

Same class as the two dead-end controls from earlier today: an affordance that
promises something it cannot deliver. The gate refusing is correct; the advice
attached to it was not.
## The private display, second pass: the three modals, and five dead ends

Took over from `docs/ui-verify-handoff.md` at `b52cfcc`. The environment was
gone (`/tmp/opencode` empty), so the first hour was the bring-up; that is now
one command, `scripts/ui-shoot/env-up.sh`, and the two traps in it are written
down at the top of the script.

**The gate claim is verified.** Against local Minos the Players island's
checklist quotes the server verbatim: *"the planned window is incomplete (both
actual_end and assumed_end are required, and each must be after its own start)"*
and *"no units assigned"*. `GET /games/2/readiness` answers the same two
sentences. This is the published gate working end to end.

**All three modals have now been rendered.** Player picker, Fleet picker and
Scenario composer, each opened, read and closed from the running console. The
composer's write path works: a step typed into it came back from
`GET /games/2/scenarios` as a third step on *First light*.

### Fixed, each confirmed by driving

**The close glyph did nothing, on every island and in every modal.** It lit on
hover and swallowed no clicks. The cause is egui's, not ours: a click goes to
one widget, the topmost clickable one whose interact rect contains the press,
with ties to the last registered. Hover is not exclusive. The close button was
registered before the body scope, so the body's own widgets were on top of it.
Registering it after the body fixes both primitives. Instrumented before the
change (`hover=true click=false`) and after (`click=true`, probe pixel flips),
because a fix for "the button does nothing" is worth exactly as much as its
evidence.

**A seated game rendered as an empty one.** `parse_roster` called `as_array()`
on `data`, but `GET /games/{id}/participants` answers `data: {participants:
[...]}`. `None` became an empty roster, with no error anywhere: the island said
`0 SEATED` while the seat was in the database and still in the API's answer.
Both shapes are read now, with a test for each and one for an envelope that is
neither. This is the third parser in this project to drop a whole field, and
the status doc already says the habit that catches them is to keep the whole
response.

**Signing out was a one-way door.** `sign_out` never raises `show_login`, and
the floating Login island is suppressed whenever the zone owns the left edge,
so the console sat at `NOT SIGNED IN` with a Sign out button and no way to sign
in. Relaunching was the only exit. The form is now one function, rendered by
the zone's Operator island when there is no identity. Sign out, sign in from
the island, console back.

**A cold start could not re-open a session.** The session picker is drawn by the
Essentials island, which renders only while a session is held, so with nothing
held the only verb was *create one* — under a top band that says "Choose or join
a session". Three sessions existed on the server and none was reachable. The
picker row is now shared by both surfaces and drawn by the no-session island
too. Re-opening yesterday's exercise is the common case; making a duplicate is
the expensive mistake.

**A sign-in that fetched nothing started blind.** `sign_out` clears the session
list, and signing back in never re-read it, so the picker row vanished on the
second sign-in. `apply_login` re-reads it.

**The Player picker opened on an empty directory.** `open_composer` loads the
book on open; the picker set a flag and nothing else. The Minos request log is
the proof — pressing the button issued no `GET /users` at all — so the read
only happened when the operator found the `find` button. It reads on open now.

**"No accounts — sign in, then refresh."** Shown to a signed-in operator whose
own account was in the directory, because the empty state was hardcoded and the
directory had not been read yet. `users_status` already carries the reason, so
the empty state says that instead of inventing a cause.

### Not fixed, and why

**A zone island's close glyph cannot close it.** `side_zone` builds its entries
with a literal `true` for open, and the draw loop sets `let mut open = true` per
frame and discards `island_owned`'s return. The click lands and is thrown away.
This one is a decision, not a bug: `DESIGN.md` gives the side zone no close
button and says its contents are a function of `GameState`, so either the glyph
goes or the zone grows a way to reopen what it closed. The second is a product
answer, and it is the owner's.

**The prose truncation does not reproduce.** At 2x on a settled frame the label
reads *"joining needs a seat first — a valid key without one"* / *"is refused
(ask the Game Master)."* The `is` is at the start of line two and nothing is
clipped. Not patched, because there is nothing to patch. The widening
experiment the handoff proposed is not worth running against a defect that is
not there; what is worth knowing is that this session independently produced a
capture with a modal's background missing entirely, which is what a mid-frame
grab looks like, and that is the likeliest explanation for the original
sighting.

**The map does not corrupt on resize.** The window is hardcoded to 1040x640 at
launch, so a capture of a 1600x1100 root photographs the black desktop beside
the app and reads as a corrupt map. Resized to 1200x900 while running, the map
fills it cleanly. Item 4 of the previous pass is closed, and it was never an
app defect — it cost a full bug hunt, which is why it is written down here.

### Still unverified, and why

**Both drag-to-map gestures.** The local mirror has no units (`units 0` in the
sync note), so the Fleet picker's rows are empty and there is nothing to drag.
The gestures need a seeded unit catalogue, which is a data task.

**Legibility of the top band over a bright map.** The band has no fill of its
own, so `0:00:00 to 1x PAUSED` in the heat accent sits on the sea while the
map's own labels run through the hint text. Whether that is intended is a
design question about the band, and it is recorded rather than decided.

## The private display, third pass: nothing in the zone is out of reach

Took over at `9e98784`. The one item the second pass named as unfixed and
structural — island bodies could not scroll, so clipped content was
unreachable — turned out to be two defects stacked, and removing both is a
deletion.

**What the operator could not reach.** At the app's own default 1040x640, in
Planning, the Essentials island measured ~760pt of content against a 380pt
island. The body was laid into a fixed rect and **clipped**, and the zone's
wheel scrolled the **column**, which moves whole islands. So the composer
button and everything below it were not on screen, and no gesture reached them:
scrolling the column moves Essentials, and Essentials is the island whose
contents are missing. At 1200x900 the island measures taller, the fold moves
down, the button appears, and the composer opens — which is why the previous
pass recorded the defect as a layout curiosity rather than a dead end.

**Root cause, two halves, and the second one is the interesting one.**

1. Bodies clipped. `chrome::island_owned` laid the body straight into the
   island's content rect. Sound as paint (a `max_rect` tells the layout where
   to stop but not the painter, so the overflow painted over the map and
   half-occluded under the next island's title band) and broken as a surface,
   because clipped content is unreachable content.

2. The zone grew every island to whatever its content measured, one frame
   behind, clamped to the window so a long body could not push the column past
   the screen. That was the **workaround for half of problem 1**, and it is
   what made the island 624pt tall against a 640pt window. An island taller
   than the screen cannot be scrolled to by any means either.

So the growth is not merely redundant once bodies scroll. It is the second half
of the defect, and the ceiling that seemed to make it safe is what made it
unfixable. Removing it is what `DESIGN.md` has always said: *"An island does
not resize to fit its content. The body scrolls."*

**The wheel is chained, not exclusive.** Every body now scrolls inside its own
rect, and the column still scrolls as a whole, so one gesture has to move
exactly one thing: the body under the pointer if it has room in the direction
being scrolled, and the column otherwise. Exclusive would have been simpler to
write and wrong — the column is the only way to reach the islands below the
fold, so a body that cannot scroll must hand the gesture back rather than
swallow it.

`chrome::BodyFit` is what makes that decidable from the caller's side:
`content_h`, `viewport_h` and `offset`, published per frame by the island that
laid itself out. A caller cannot work any of it out for itself, because the body
is laid out inside `island_owned`.

### The slack that keeps four points of nothing from eating a gesture

A layout's measured extent runs a few points past its last control for trailing
spacing. The Operator island measures 98.4 against 94 available: 4pt of nothing
at the bottom. With a zero tolerance that 4pt is scrollable, so the Operator
body claims the first wheel gesture, moves four points the operator cannot see,
and the column does not move — a gesture that appears to do nothing, which is
the exact failure the chain exists to prevent.

`BODY_SCROLL_SLACK` is 12pt, the same tolerance and for the same measurement as
the `OVERFLOW_SLACK` this replaced, and it is pinned by a test on both sides:
below 4pt the slack claims gestures for trailing spacing, above a text line it
hides real content below the fold.

### What was deleted, and why that was the smaller change

`island_scrolled`, `island_fitted_height`, `fitted_island_height` and
`OVERFLOW_SLACK` are all gone, plus `ISLAND_SCROLL_MAX` in both `tokens` and
`main.rs`. Every one of them was a way of saying "the body scrolls" or "the
height adapts" in a second place, and every one of them was reachable by
mistake. The six `island_scrolled` call sites in `main.rs` and three in the
prototype crate now call `island`, which scrolls unconditionally.

The diff removes more than it adds in `chrome.rs`, and the whole of the
`main.rs` change is deletions. The scroll discipline `DESIGN.md` specifies —
*"island bodies scroll at a 420px cap so a tall island never swallows the map"*
— is now what the code does rather than what the code approximates.

### A second defect, found by driving, not by reading

`composer_modal` handled its close by clearing the selected scenario and the
draft, and never set `composer_visible = false`. The modal therefore stayed on
screen, re-rendered with an empty book, and had no way out. The flag is written
in `open_composer` and by `apply_login`; this was the only other place that had
to write it, and nothing about reading `composer_modal` suggests it. Found
because the harness clicks the composer's ✕ and then asks whether a modal is up.

### Verification

`scripts/ui-shoot/verify-scroll.sh`, seventeen checks, three consecutive clean
runs. The invariant it asserts is per frame, over the numbers the app printed:
**the column offset moved only when no body under the pointer had room**. Scoped
to the pointer deliberately — "some island had room" is true on every frame,
and a checker phrased that way reported fifteen violations per gesture against a
chain behaving exactly as designed.

Alongside it, and each confirmed on a render: the composer button is located in
the frame, clicked, opens the composer, and the ✕ closes it. The wheel over a
gap moves the column. The wheel over a body that fits moves the column. The
wheel over the map moves neither. And with the column back at the top, the body
takes the wheel again.

`--lib` 300 pass with the same two pre-existing failures as before this work;
`--bin tfg` 44 pass; four new chrome tests pin who owns the wheel and the
slack.

### What the tools cost, and what they are now

Every coordinate read off a rendered screenshot was wrong by 10 to 30pt,
because a cropped image is displayed at a scale that is not knowable from the
image. Three rounds went into clicking the wrong control. `shot.sh` burns a
labelled grid into the capture; `click-sweep.sh` and `hover-sweep.sh` use one
probe pixel as an oracle instead of reading a picture. `xdrv click()` now holds
the press for 70ms, because a zero-length press+release is dropped often enough
to read as a dead control. And the app needs 30 seconds after launch before it
will take input at all — 14 is not enough, and the symptom is silence.