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
| The Fleet Picker modal | `ShipApp::fleet_picker_modal`, geometry in `tfg::picker` | `cargo check` clean; **not looked at** (no compositor). The geometry (column widths, panel size, strip, drop rule) is kernel-tested with no egui context |
| A clear backdrop for drag-to-map | `chrome::Backdrop` | 2 tests: it drops the dim and nothing else, and a dim is actually dimmed |
| A release over a panel is not a drop | `picker::drop_lands_on_map` (moved out of `main.rs` for the GPUI cutover, ADR-0020) | 3 tests: the panel edge half-open, the empty case, and TWO panels where one would pass both |
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
## The GPUI shell, fourth pass: a hull's picture at its own size, and a fleet the island can admit

Took over the GPUI cutover with two complaints from the operator, both read off
a driven window: at `z14` a deployed hull still drew its NATO glyph instead of
its photograph, and the Fleet island said `0 PIECES` while a hull the operator
had just placed stood on the map. Both were real, and neither was where it
first looked.

**The picture was never reachable.** The map's thumbnail is sized by the hull's
published length and beam, and the previous pass joined those figures into the
fleet taxonomy from the local `unit_specs` mirror. That mirror is filled by
`sync_spec`, which ran for `session.pieces` only — and in a *planning* exercise
the hulls that get drawn are in the operator's **draft**, which the server has
never heard of. So a fresh exercise had no dimensions for exactly the hulls on
the map, `sizes()` came back empty, and every marker fell through to the glyph.
The handoff's log check ("hull 1's spec synced") was true and beside the point:
hull 1 was in a *different* session's list at the time.

The fix deletes the join rather than widening it. Minos' unit-image **manifest
already declares `loa_m` and `beam_m` beside the picture** — the same entry the
egui console's `UnitVisual::from_entry` read — so the picture and its
proportions now travel together to the shell in one `ThumbRaw`. One request
covers every pictured hull, no store read is involved, and the map is sized by
the same fact that supplied the image. `FleetTaxonomy::sizes`, the taxonomy's
two leaf fields and the `build_fleet_taxonomy` join are gone; a hull with a
picture but no published figures still draws its glyph, which is the honest
answer and is pinned by a test.

**The size was in map pixels, painted in window pixels.** Marker positions are
scaled to the window (`sx = win_w / map_dims.0`) but the thumbnail's length and
beam were not, so a resized window drew a hull at the wrong size against the
water it sits in. The picture is now `loa / mpp × sx` by `beam / mpp × sy` — the
frame is drawn with a `Fill` fit, so the window's own stretch IS the ground's
stretch. `portrait()` is the one function that decides "photograph or glyph" at
a zoom, and its refusals (zoomed out, no picture, no published size) are what a
test asserts.

**The Fleet island listed the server, not the exercise.** `fleet_body` iterated
`session.pieces`, so a hull deployed locally — in the draft, not yet written
through the advance — was invisible there while the picker showed it. The island
and its band now read `deployed_pieces()`, the one merge: the session's rows with
the operator's newer edits laid over them, then the draft's own hulls, ordered by
name so the list cannot reshuffle between frames. The picker's `IN THE EXERCISE`
count uses the same merge, so the two cannot disagree.

Two smaller versions of the same defect fell out of driving it:

- The picker's drill rows decided "assigned / place" from `session.pieces`, so a
  hull the operator had just added had no `place` pill at all. Now `in_exercise()`.
- The armed click then posted directly and gated on `session.pieces`, so the
  `place` pill it had just offered answered *"that hull left the exercise"*. The
  armed click is now the SAME verb as a drop (`drop_unit`): staged while
  planning, posted afterwards. One row of the matrix, not a second rule.

### Verified on the private display

Driven at 760x620 on `:99`, a planning exercise with **zero** server pieces (the
seeded six removed for the run and restored after), the hull added through the
picker and placed by the armed click:

- Fleet island band: `1 PIECES`; body: `FLEET PICKER (1 IN THE DRAFT)` and one
  row, `KRI Ahmad Yani · placed`. The picker shows one already-synced hull; the
  island shows it because the draft holds it, which is the whole point.
- `z13`: the NATO glyph, in its frame, at the marker.
- `z14`: the photograph, measured on the capture at **25 x 4 px** — the spec's
  120.5 m x 16.2 m at Jakarta's 4.75 m/px, `z14`, times the window's stretch.
  Not a square, not a glyph.
- `z15+`: the marker leaves the viewport, because the camera centre is preserved
  and zooming magnifies the offset from it. That is projection, not drift.

`cargo test --lib`: 545 pass, the same four pre-existing failures
(`book::prune_drops_relations_for_vanished_drafts`,
`map_render::quad_bow_follows_the_compass`,
`roster::seats_come_before_roles_before_unseats`,
`sim::order_move_applies_then_higher_overrides_loudly`).

### Not verified, and why

- **The live-marker path.** A server-placed hull takes the same `portrait()` call
  and the same sizing, but there is no live wire on this machine (`ws://` is
  refused; replay is empty), so no fix-fed marker was ever photographed at the
  zoom. Unit-tested; not seen.
- **Rotation.** The egui path rotates the photograph to the hull's heading
  (`rotated_unit_quad`). GPUI 0.2.2's `paint_image` takes an axis-aligned
  `Bounds` and no rotation is available, so a course-bearing hull would need a
  quad painting path. Not attempted, and not promised.
- **Legibility.** At `z14` a 120 m hull is 25 px long and 4 px tall: true to
  scale, and too small to read a silhouette. From `z16` (~100 x 14 px) the
  picture reads. If the console wants the egui ladder's rule instead — draw the
  picture only once its footprint is worth reading — that is a threshold change
  in `portrait()`, not a new mechanism.

### The trap that reads as a hang

A freshly mapped GPUI window has not presented a frame, and `import -window`
captures it **black** until X sends it an Expose. Focusing the window is what
triggers the first present — which is why a launch can look hung for minutes and
then paint the instant a click lands on it, and why one earlier pass recorded a
flaky launch that never rendered. `xdrv focus` only looked for the *egui*
window, so against every GPUI build it answered "no egui window" and did
nothing. It now falls back to the `tfg` window, and the first thing a driven run
should do after `xdrv list` prints an id is `xdrv focus`.

## The GPUI shell, fifth pass: why the hulls were blurry, and a door that centres one

Two reports off a driven window at `z18`: the hull picture is very blurry, and
the ships face the wrong way. A third ask: a per-row control on the Fleet island
that frames that hull.

**The blur had one cause and it was ours.** `spawn_thumb_fetch` decoded each
picture with `img.thumbnail(192, 192)` — a real thumbnail, kept from when the
only consumer was a picker row. The map, meanwhile, paints the picture at the
hull's REAL size: a 120 m destroyer at the console's own `ZOOM_MAX` (18) is
about 400 px long, so a 192 px bitmap was upscaled 2x exactly when the operator
zoomed in to look at it. The fix is to keep the picture's own resolution, capped
at 2048 px on the long side so a 6000 px upload cannot cost tens of megabytes per
hull; the rows, the ghost and the banner downscale it themselves. Confirmed on a
capture at `z18`: the box is 1:1 with the 400 px source and there is no
interpolation left to see.

**A picture's own orientation is now applied on decode.** `image::load_from_memory`
ignores EXIF `Orientation` by design, so a photograph stored as "rotate 90" —
the normal way a phone or a camera records that it was turned — decoded on its
side, and every consumer inherited it. A top-down ship on its side on a north-up
chart reads as a hull steaming north, which looks like a projection bug and is
not one. `decode_picture` now reads the tag off the decoder and applies it once,
before the picture goes anywhere. Pinned by a test that encodes a PNG with a
hand-built EXIF block (`Orientation = 6`) and asserts the decoded dimensions are
the quarter-turned pair, plus the no-tag and "orientation 1" cases that must be
no-ops.

**Rotation to the hull's course is now implemented, on the CPU.** The egui
client rotated the photograph to the hull's heading through `rotated_unit_quad`.
GPUI 0.2.2 offers no equivalent at paint time: `Window::paint_image` takes only
`bounds`, `corner_radii`, the image and a frame index — the scene's
`PolychromeSprite` has no transformation field, and the only element that can
rotate is `svg` (`with_rotation`). So the map turns the RASTER instead, in
`thumb_rotate::turn_to_size`: the picture is sampled with an inverse map
straight into the box it is drawn in, so the console never holds a turned
picture bigger than the box it goes into, and the turning is a sampling
rather than a rotate-and-resize. `IMAGE_FORWARD_HEADING_DEG` (bow to the
right) is the axis convention it honours: the picture is turned by the course
less a quarter turn, so a hull on 090 is drawn exactly as the asset arrives.

The turned pictures are cached per hull, per box and per whole degree, and
emptied past a byte budget — a drag across a course is a turn a degree at a
time, and the sprite atlas would otherwise keep every one of them.

**And the NATO symbol is NOT turned.** The affiliation frame and the taxonomy
glyph are the console's own symbology and they stay on the axes: the frame
says which side the hull is on, and a frame that turned with the hull would be
a frame saying which way the hull points, which is the needle's job.

**The fleet row's focus door.** Each piece row now carries a reticle at the row's
right end (between the name and the placed/unplaced word) that asks the camera to
frame THAT hull. `piece_ground` answers "where is it": the fix when the wire has
reported one, otherwise the start the operator staged; the mark is drawn dimmed
and takes no click for a hull with neither, because a door that frames `(0, 0)` is
a lie about where the hull is. `jump_to_selection` (the Inspector's `Centre`) is
now the same verb addressed by hull, so the two cannot disagree.

**And a sign error that had been hiding the whole feature.** The binary's jump
handler unprojected `mw/2 + shift_px` from the hull, which moves the camera
*toward* the hull's east and parks the hull `shift` pixels to the WEST of the
frame centre — under the floating zone, which is the half of the frame the shift
exists to clear. Both the Inspector's `Centre` and the new focus door therefore
appeared to do nothing at all: the camera moved, and the hull it should have
framed sat behind the column, invisible. The shift is the CONTENT's move, so the
camera moves the other way (`mw/2 - shift`). `jump_goal` is now a named function
with a test that projects the hull back through the goal and asserts it lands on
the visible centre for a left dock, a right dock and no dock.

Verified on the private display: the door was clicked at `z18` and the hull
appeared at `x = 560` — the frame's centre (380) plus the 180 px the left dock
shifts content by — with its picture at native resolution.

`cargo test --lib`: 546 pass, the same four pre-existing failures.
`cargo test --bin tfg`: 5 pass, two of them new (the EXIF decode and the jump
sign).

## The Scenario board, and a pin for the composer

Three things the owner asked for, and one of them moved a surface out of the
column.

### The board is in the middle, because a scenario is not a 300pt panel

The `STEP` island is GONE. It carried the running step, the readiness gate and
the declare row, and it was honest as far as it went — but a scenario is the
whole book, which of its steps have been played, the hulls each step obliges,
the gate, and (for the Game Master) the verbs that move it. Four of those fit a
column; the rest were either missing or below the fold, which is a panel saying
the plan is smaller than it is.

`board_body` is one card in the centre of the frame (`BOARD_W`x`BOARD_H`, the
authoring silhouette in its own size) with a row per step of the whole book in
play order. Each row carries its number, its state, its readiness gate, its
words, its window, and the hulls the server resolved for it (`related.names` —
the names were already in the binary's answer and were being dropped, so the
board cost no new read). The step the exercise stands on is lit with the cyan
edge and the word PLAYING; the row's verbs sit after the words they act on.

Two doors, and neither is the column: the band's `SCENARIO` button (execution
only — the plan is written in the Book, played here) and the reminder strip,
whose click used to scroll the column to an island that could be below the fold.
Both land on the same surface; the strip's capture is pixel-identical to the
band's.

`zone_islands(Execution, …)` is now `[Operator, Command, Crew, Log]` for an
organizer and `[Operator, Command]` for a seated player. `IslandKey::Step` is
deleted rather than left unpushed: a key with no body is a slot waiting to be
filled by the next person who reads the enum.

### Three audiences, one model

`scenario_board()` builds the rows once and the board draws them; what differs
between the seats is what each row may press. The audience comes from the SEAT,
not from a preference:

| seat | audience | sees | presses |
| --- | --- | --- | --- |
| no seat, `Game Master`, `Controller`, `Deputy` | Master | every step, its hulls, the pointer, every gate | PLAY · SKIP · END · REPLAY, and the clock |
| `Observer`, and every other judge-side seat | Observer | the same rows and the same pointer | nothing |
| `Commando`, and any seat that takes part | Commander | the ONE step their station owes | DECLARE READY / WITHDRAW |

The Game Master is a *participant* seat in Minos (`is_judge_side: false`, "Runs
the exercise"), so the console's existing `player_station()` rule would have read
them as a commander. The role names are matched because that is the only
machine-readable thing Minos says about who runs an exercise — the same way the
clock grant is described.

The four verbs are `ToBinary::StepVerb`, one message for one vocabulary, and the
binary calls the routes Minos has published all along (`/play`, `/skip`, `/end`,
`/rewind`). The earlier handoff said these were "un-aimable" without a runs read;
that was wrong, and the correction is now in the handoff itself. What a runs read
would add is HISTORY, not aim: the step a verb names is the row the operator
pressed, and the server has the last word on the seat.

A `rewind` answers with what the erasure COST instead of a run, so `CreatedSession`
grew a `note` — the board shows "step 4 replayed — erased 2 order(s), 0
report(s), … " under the verbs. A Game Master who just erased an attempt is
entitled to be told what it took.

### Played, and the one place the board needs a memory

"Played" is derived: a step is behind the exercise if it sits before the pointer
(`current-step`), which needs no history to claim. `end`, though, takes the
pointer AWAY — the server stops naming any step, and a book that looks untouched
is what comes back. So the shell keeps `step_seen`: the steps this console has
actually WATCHED in play, noted once per frame in `render` (a dozen drains install
the session, and a ledger written from each is one with a dozen chances to miss
one).

It is labelled as observation, not as the server's record, and it is corrected by
the server wherever the server can speak: a step the GATE is asking about is not
behind the exercise, so the ledger is truncated at that step — which is exactly
how a `rewind` (this console's or another's) is learned, since rewinding puts a
step back to un-reached and asking its seats again. A step the console never saw
stays LATER rather than being assumed done.

After an `end` the board therefore reads `1 of 4 played · between steps`, shows
the finished row as PLAYED with REPLAY, and arms the first unplayed row with
PLAY and SKIP. One row is armed at a time, always: playing the step after the
pointer would let one stray press jump the plan.

### The observer's replay, said plainly

An observer reads the whole plan and the pointer, and the board says why that is
all it can be: replaying a played step's HISTORY needs the runs behind it, and
Minos publishes only `current-step`. Rewinding the exercise to watch it again is
the other reading of "replay", and it ERASES other seats' orders and reports —
so it stays with the Game Master. The foot of the observer's board says both
sentences instead of offering a button that would lie.

### The pin

The Book's step row grew a `PIN`. Armed, the Book stands aside (a modal over the
map is a form with nothing to click), a cyan strip says what the next click will
do — and IS the way out, because GPUI 0.2.2 has no Escape binding here — and the
map takes the click. The click is unprojected locally (`frame_unproject`, the
same inverse the planning drops use) and the coordinate is written INTO the
step's words: `Take station east of the buoy at -6.1047, 106.9895`.

Four decimals, deliberately the same `{lat}, {lon}` the Inspector reads a fix
back with: a position quoted off the map into a step and the position the
inspector shows for that place have to be the same digits, or an exercise is
authored against one number and reviewed against another. Four decimals is about
eleven metres; the pin says "here", not "this rivet".

The pin is TEXT and nothing else — no shape parked on the map. That is what was
asked for, and it needs no storage question: the step's content is the record.
A second pin appends rather than replacing, so an instruction can name two
places. Any other door in the console cancels an armed pin, because a mode that
survived the operator walking off would make the next map click quote a
coordinate nobody asked for.

### Verification, driven on `:99`

A game was built through the API for each audience (`SHOOT-01` for the pin,
`COMMANDER PROOF`, `OBSERVER PROOF`, `DECLARE PROOF`), advanced
planning → preparation → execution with readiness declared, and the binary was
driven with `xdrv` and captured.

| claim | capture |
| --- | --- |
| the pin lands in the composer's words | `b2` (typed), `b4` (armed: strip + Book gone), `b5` (`… at -6.1047, 106.9895`, Book back), `b6` (the step posted with it) |
| the Game Master sees the book, the gate and the hulls | `d1`: `not started · 4 step(s)`, `0 OF 1 READY`, `KRI Ahmad Yani · held by LAKSAMANA` |
| PLAY lands and the pointer moves | `d2`: `step 1 of 4`, PLAYING, END/REPLAY, `1 OF 1 READY` |
| a refused PLAY says why | `c2`: `step play refused: HTTP 409 … 1 of 1 participant(s) related to this step have not declared themselves ready.` |
| END leaves "between steps" and arms the next | `d3`: `1 of 4 played · between steps`, row 1 PLAYED + REPLAY, row 2 PLAY/SKIP with its gate |
| REPLAY reports the erasure | `d4`: `step 4 replayed — erased 2 order(s), 0 report(s), …` |
| the commander sees one step and declares | `q1` → `q2`: `ASKING`, `0 OF 1 READY`, DECLARE READY → `1 OF 1 READY`, WITHDRAW |
| the commander sees no other row and no GM verb | `k1`: `step 1 of 3`, one row, PLAYING, no verbs |
| the observer sees everything and presses nothing | `w1`: three rows, the pointer, the gates, no verbs, the replay sentence |
| the reminder strip opens the board | `w5` is pixel-identical to `w1` |

Unit tests, all in `shell_gpui::tests`: `the_board_reads_the_whole_book_and_where_the_exercise_is`,
`a_fresh_execution_arms_the_first_step`, `the_board_carries_the_gate_and_its_verbs_on_the_asking_step`,
`an_observer_reads_the_plan_and_presses_nothing`, `a_commander_sees_only_the_step_that_asks_them`,
`a_game_master_drives_a_step_through_the_one_message`,
`a_watched_step_stays_played_after_it_ends`,
`the_ledger_notes_played_steps_and_forgets_a_new_game`,
`a_pin_writes_its_coordinate_into_the_step_text`, and
`opening_the_reminder_opens_the_scenario_board` (which replaced the island-reveal
test the move retired).

`cargo test --lib`: 554 pass, the same four pre-existing failures.
`cargo test --bin tfg`: 5 pass.

### The pin you can pick up and drop

The arm-then-click pin is still there and is no longer the only gesture: the
Book's `PIN` is now a **yellow pin you DRAG onto the water**. Dragged, the Book
stands aside (the same `dragging` flag the Fleet picker hides on), the pointer
carries a small yellow `pin · quote` ghost, and the release point is unprojected
and written into the step's words. Clicked, it arms exactly what it used to.
Both paths go through one act (`quote_pin`), so the drag and the click cannot
quote two different numbers for the same water — pinned by
`a_dropped_pin_quotes_what_an_armed_click_would`.

The mark is drawn, not typed: four quadratic curves and a filled hole in
`pin_glyph`, in the theme's `ALERT_YELLOW` (#FFFF00) — the one deliberately loud
colour in the console, because it is a thing you pick up rather than a status.
`PinDragPayload` carries nothing at all: a pin is not an entity, so its TYPE is
the whole message and the map's second typed `on_drop` is its only reader. A pin
that travelled as a `UnitDragPayload` with a flag would have answered the entity
drop vocabulary, and every row in the console would have accepted a gesture that
means none of them.

**Two defects the drag exposed, both older than it.** GPUI delivers a press to
EVERY hovered hitbox down the chain, so a press inside a card also lands on the
full-window map surface underneath it — which sets the map's pan handle. The
fleet picker had already recorded this and blocks it with
`block_mouse_except_scroll`; the Book's card and the Scenario board's now do the
same. And the pan itself: a gesture that leaves a card started life with a pan
attached, so the map slid out from under the ghost and the drop named ground
that had moved. `set_dragging(true)` now clears the pan handle, because a press
that became a drag is not a pan.

Verified on `:99`: `r1` (the yellow control in the Book), `t6` (mid-drag — Book
gone, the `pin · quote` ghost under the pointer), `t2` (dropped: `at -6.1675,
107.0141`, and the map did NOT move), `t4`/`t5` (a second drop north-west quoted
a point north-west of the first, so the release point is what is read), `u1`
(clicked: the armed strip), `u2` (the armed click appended a fourth coordinate).
The Book's field still takes typing after the block (`u3`).

### The commander's rail, and the compass that pointed the wrong way

The commander's dashboard was a **rounded, translucent panel floating eight
points off the bottom of the window**, and it was ugly for reasons that had
nothing to do with taste:

- The tile map's own labels came through the type that sat on them. A
  console read across an exercise room, under glare, cannot put its readings
  on top of a map and call it legible.
- The panel was a card, and a card that is 1904pt wide with 800pt of content
  is a card with a void in it. The right half of the strip was the map,
  showing through.
- The course compass **did not render**. It was a cyan smudge about twenty
  pixels across, in the wrong corner of its own box, pointing at a bearing
  the readout beside it disagreed with.
- The speed control was a 110pt slider — a knot every three points.
- The identity rows were laid out inline, so each value started wherever its
  label ended: the hull number under `HULL`, the group name under the group
  icon, the embark line under the anchor. Three values, three columns.
- The photograph sat in a portrait box, letterboxed, with three quarters of
  the frame empty.

**The compass was the interesting one, and the cause is a GPUI rule worth
writing down.** A `canvas` is a **block-level** element, and so is a `div`.
A block child of a block parent is measured at its content's height, and a
canvas has no content, so **a canvas inside a block parent is measured at
ZERO HEIGHT** unless it is given a size. The dial's canvas sat in
`div().w(80).h(80).relative()`, so it was measured 80 × 0. The radius came
out at `min(w, h) / 2 - 2` = **−2**: the ring collapsed, the graduations
shrank to a few pixels around the hub, and the needle — drawn from the
canvas's own top edge rather than the dial's centre — pointed wherever the
top edge was. The pointer maths used the *div's* bounds, so the readout was
right and the needle was somewhere else entirely: a control that reads 34°
and draws north. The canvas now carries `.size_full()`, and the face is
measured about the same centre the pointer maths uses.

The same idiom was checked everywhere else: every other `canvas` in the
console either carries an explicit size (`.size_full()`, `.size(px(14))`)
or paints from absolute coordinates rather than from its own bounds. The
dial was the only one that was broken, and the only one that read its own
bounds for geometry.

**The rail.** Opaque, square, flush with the window's edges, one 1px
`CUT_GREY` rule along the top — the map ends at the rule instead of running
under the chrome. Three channels, separated by 1px rules: the picture and the
name, the identity rows, and the helm. The rail's widths are a pure function
of the window's width (`rail_widths`), so the layout the operator sees and
the layout the test checks cannot drift apart.

**The helm is an instrument now.** A course is a circle, so it is a compass:
a rim, twelve graduations with the cardinals longer, a north wedge, and a
needle with a hub and a short tail. A speed is a range, so it is a
**graduated scale**: a track, the value's ink extent along it, a tick every
two knots with a long one every ten, and the numbers under it — five knots
apart on a long track, ten on a middling one, because a number every 275
points on a 1100pt scale is three stray labels rather than a scale. The
scale's track sits at the compass's **hub height**, so the two instruments
share one horizon instead of floating at unrelated heights, and the labels
for both are on ONE row with the controls under them (the first version put
each control's label in the control's own column, and a compass 80pt tall
next to a slider 22pt tall put the two labels at different heights).

**The speed scale takes the slack.** The rail's width goes to the one
element that gets better with length; a knot every three points is a slider,
and a knot every twenty is an instrument. The compass and the scale are
therefore the last things to shrink on a narrow window, and the picture and
the column of facts give way first.

**The instruments settle.** A needle that teleports between two bearings
reads as a glitch, so the compass eases onto a new draft over 220ms with an
exponential ease-out, and the scale's fill eases with it. The bearing eases
the **short** way round, so 350° to 10° is twenty degrees forward. A
pointer **drag** drops the ease entirely: while the operator is holding the
control the instrument IS the hand, and anything that trails the hand is
lag, not motion. The console's own frame loop is the map's — half a second
a frame on a software rasteriser — so the settle asks for frames of its own
(`window.request_animation_frame`).

**The picture is framed at its own proportions.** A hull plan is four to
one, and a four-to-one picture in a portrait box is a postage stamp in a
vitrine. The tile is now as tall as the picture is wide-by-tall, and it is
square, because nothing on an instrument is rounded except the controls you
turn.

Verified on the private display, driving the real binary against the
deployment the operator uses (`api.tfg.development.crossnet.co.id`, session
`DEMO-SCENARIO-UI`, signed in as the commander who holds `KRI Ahmad Yani
(KRI-351)`), at 1920 × 1150 and at 760 × 620. The captures are in
`.impeccable/review/`: `rail-before.png` (the strip as it was),
`rail-after.png` (the rail), `rail-helm-dragged.png` (the two controls after
a drag) and `rail-order-refused.png` (the answer to a real order).

| | what the capture shows |
|---|---|
| the rail | opaque, square, full width, one rule along the top; the map ends at the rule |
| the compass | a rim, twelve graduations, the north wedge, and the needle at 0° — pointing UP, which is what 0° means |
| the compass under a drag | dragged east: the readout says **77°** and the needle points **77°** — the two agree, which they did not before |
| the scale under a drag | dragged to 13.1 kn: the figure reads **13.1 kn**, the cyan fill runs to the thumb, and the thumb sits on the 13 graduation |
| the identity column | `HULL / KRI-351`, `GROUP / Gugus Alpha`, `EMBARK / nothing embarked` — one right-aligned column, three hairlines |
| the picture | the hull plan at its own proportions, square frame, name under it |
| 760 × 620 | the compact rail: the picture and the facts shrink, the compass and the scale do not, and nothing overflows |

**The write, verified against the local stack** (a throwaway database, so a
real order could be posted without touching the deployment the operator
uses). Game `DASH-11`, a Commando seat holding `KRI Ahmad Yani (DD-1002)` in
`Gugus Alpha`, in execution. The helm drafted 67° / 9.6 kn, `SET HELM` was
pressed, and the server answered:

> refused: HTTP 409: Conflict occurred — This unit's maximum speed is unknown,
> so the order cannot be clamped and was not applied. Publish one for the
> medium it moves in, with PUT /units/{id}/speeds.

which the rail showed, in the refusal's ink, in the space under the verb.

**And the answer had to be bounded to be readable.** The server's refusals
are SENTENCES, and a sentence in a column this narrow is more lines than the
rail has height for: the first version of the answer ran off the bottom of
the band and was cut off by the window. The answer is now cut to the room
the rail has under the verb — three lines — with an ellipsis, and the part
that names the problem is the part that is kept. The rail's height is
therefore sized for three lines of answer, and the answer can never be what
pushes the band over.

Unit tests, all in `shell_gpui::tests`: `the_rail_gives_the_scale_what_is_left`,
`the_scale_numbers_sit_on_their_graduations`,
`the_helm_instruments_settle_on_the_draft` (which pins the ease-out, the
short way round the compass, and that a drag drops the settle) and
`the_order_s_answer_is_cut_to_the_rail`.

`cargo test --lib`: 578 pass, the same four pre-existing failures.

**A defect found on the way, not fixed here.** A seat that is not an
administrator **cannot sign in at all**: `fetch_roster` reads
`GET /units?page_size=200`, which the server answers 403 for anyone without
the administrative grant, and the sign-in reports the refusal as *sign-in
refused — check the identifier and password*. The credential is right and the
console says it is wrong. The commander seat this rail is built for is
exactly the seat that cannot get in. It is a one-line answer — the roster is
the console's own unit catalogue, and a commander needs their own units, not
the whole register — but it is a product answer about what a non-administrator
sees on the sign-in card, so it is recorded here rather than changed.

## The game mechanics: a commanded hull that actually MOVES

The operator's complaint was one sentence: the previous EGUI console's backend
worked — *the units, if commanded, move, as the backend emits through the
websocket* — and the GPUI console did not. He was right, and the reason is
worse than a missing feature: **the GPUI console had no game-mechanics path at
all.** The live socket's events were received into a channel and dropped on the
floor (`let (event_tx, _event_rx) = mpsc::channel();`), the game channels were
never watched, and nothing ever read the exercise's plot. A hull could be
ORDERED from the rail — that half works, and the previous pass proved it — and
then it stood still on the map forever, because nothing was listening for where
the server had put it.

The kernel was never the problem. `Registry::poll_game`, the game fix source,
the trailing blend and the whole `Fix` model were all ported and all correct:
the egui console's `ingest_game_positions` had become `gamefeed::game_fixes`
waiting for a caller. The GPUI binary simply never called any of it.

### What the movement is made of, and where each piece comes from

**Three sources, one seam** (`src/gamefeed.rs`):

1. **`GET /games/{id}/positions`** — the exercise's derived plot, advanced along
   the fix chain to the scenario's now. The realtime protocol names it the
   interim answer to the silent positions channel, and it is the source that
   works today: read once a second while the exercise is running, and once
   every five while it is not, so the transition into execution is noticed
   without the console being told.
2. **`game:<id>:positions`** — the socket's own position stream. The protocol
   says the channel is specified and silent, and the server here publishes on
   it anyway (the local stack's log says `service: published a position
   snapshot`, once a second), so the console subscribes and ingests it. The
   subscription is declared the moment the exercise is running, and the
   payload is the agreed shape.
3. **`game.order_issued`** on `game:<id>` — the order's own position, published
   the moment the order lands. It is the FIRST word about where the hull went,
   and it arrives before any plot read can report one.

All three become the same thing: a `Fix` with `FixSource::Game`, ingested on
the registry's GAME path — never the wire's, so the wire's silence cannot mark
a running exercise's hulls stale, and never aged against the wall clock,
because a game fix's instant is the SCENARIO clock and is routinely hours from
the wall's.

### The bug that would have made it look wired and be dead

The first version stamped each fix with the position's own `assumed_time`. It
looked right — the position carries it, after all — and it is the LEG's start,
not the round's: a hull ordered at 13:00 reports 13:00 for as long as it
steers on that leg. So every read of a moving hull carried the SAME instant,
and the registry — which orders by instant — accepted the first and refused
every one after it. **The hull stood still on the map while the server moved
it**, and the console's own log said the exercise was running.

It was found by driving, not by reading: the server's plot advanced every ten
seconds and the marker's pixel did not move by one. The rule is now the
round's instant, with a test that asserts it twice over — once on the stamp,
and once through the registry, that two rounds of a moving hull both land
(`two_rounds_of_a_moving_hull_both_land`).

### The verification, driven

Against a local Minos (Postgres, valkey, Minos and a Centrifugo v5 built from
the same tree), a game in execution with one hull placed, published with a
speed and commanded by the account that signs in.

| | what the evidence says |
|---|---|
| the order | the rail's compass was dragged to 233° and `SET HELM` pressed; the rail answered **`applied 233° / 29.9 kn`** — the server's own numbers, the speed clamped from 37.2 to the hull's published 29.9 |
| the feed | the log carries both paths, once a second: `game feed: 1 hull(s) @ … — 1 at -5.912259, 107.192129 (232.6° 29.9 kn)` (the plot) and `wire feed: 1 hull(s) on game 1 @ …` (the socket) |
| the marker | the same region of the map, 70 seconds apart, the marker **49 px west and 35 px south** — the new course, at the new speed |

The captures are in `.impeccable/review/`: `move-before.png`, `move-after.png`
and `move-side-by-side.png`.

**The console's own camera has to follow.** The hull is a few kilometres off
the map's centre, and the console has no focus door in execution, so the
camera was panned onto it by hand. That is worth saying plainly: the
movement is verified, and the gesture that makes it readable at a glance is
not there. A hull that is not near the centre of the window is a hull the
operator has to go and find.

### Two things found on the way, neither of them the movement

**The rail's answer was clipped by the window, not by its column — FIXED.**
At the window the operator's desktop gave the console (640 points wide, which
is 960 X11 pixels at the scale the compositor was running), the rail's verb
column was the last 148 points, and the answer ran past the right edge:
`applied 40° / 29.` and the rest of the sentence — and the `SET HELM` button
itself — were off-screen. The previous pass wrote that the answer is "cut to
the room the rail has under the verb — three lines — with an ellipsis"; what
happened is that the TEXT was not cut at all, and the window cut it
mid-figure. The verb is the console's primary action, and at this width it
was a 20-point sliver.

**The arithmetic was the cause, and the arithmetic was wrong.** The rail's
widths are a pure function of the window's width, and the scale was the
remainder with a floor of 48 points: when the window was too narrow for the
floors, the floor was honoured and the rail was allowed to overflow. The
overflow went RIGHT, because the verb is the last channel. So the scale's
floor was bought with the verb.

The rule now: **the rail is the window, at every width**, and the points the
scale's floor needs come from the channels that give way first — the picture,
then the column of facts, then the compass's face — and the verb is the LAST
thing squeezed, and only in a window too narrow for the rail's floors. The
floors are now stated: the picture keeps 144 points (the hull's NAME is the
one thing in that channel that is not a picture), the facts 150 (the longest
reading whole), the compass 64 (a face you can still grab), the verb 72 (the
button itself) and the scale 48. A test walks the widths from 500 to 2560
and asserts that the channels and the chrome add up to the window, every
time (`the_rail_fits_the_window_and_keeps_its_verb`).

Verified on the operator's own window, at 640 points: the `SET HELM` button
is whole, and the server's answer reads **`applied 6° / 29.9 kn`** — the
whole sentence, on one line, inside the window. The order that produced it
was the compass dragged to 6° and the verb pressed, and the hull's own course
came back on the rail as 6.1°, which is the server's answer read back out of
the fix. Captures: `.impeccable/review/rail-narrow-before.png` and
`rail-narrow-fixed.png`.

**A capture on XWayland is stale, and it reads as a dead feature.** The
X11 `import -window` of a Vulkan window on XWayland returns the last frame the
X server held, not the window's current content: two captures 70 seconds apart
were BYTE-IDENTICAL while the console was alive and the hull was moving, and
the only captures that showed the movement were the compositor's (`grim`).
Worth writing down because the failure mode is the exact opposite of the usual
one — a stale capture says "nothing happens", and the conclusion drawn from it
is "the feature is broken" rather than "the picture is old".

`cargo test --lib`: 585 pass, the same four pre-existing failures
(`book::prune_drops_relations_for_vanished_drafts`,
`map_render::quad_bow_follows_the_compass`,
`roster::seats_come_before_roles_before_unseats`,
`sim::order_move_applies_then_higher_overrides_loudly`).

## The sign-in, and the theatre behind it

The signed-out surface was the last thing in this console still wearing the
old world: a rounded card inset 24pt on two sides, floating on a flat
`CONSOLE_NIGHT` void, with a web form's hint words inside its boxes. It was
also the first thing every operator saw, which made it the one surface the
overhaul could not leave alone and call the console finished.

**THE THEATRE.** The console's own map, live, under a flat night scrim. The
map thread runs from the launch, so the picture is already on screen when
the operator arrives: the sign-in happens over the water the exercise will
be fought on. The scrim is flat `CONSOLE_NIGHT` at 0.86 — no gradient, no
blur, no glass — so the map is a GHOST under the console's own night.
Measured on the capture: the ghosted land reads `(47,53,69)` and the panel's
own body `(15,23,42)`, so the chrome is darker than the map under it, which
is the relationship every panel in this console has over the live map.

**And it ARRIVES.** The console opens on a flat night field and the
coastline resolves out of it: the scrim lifts from opaque night to the ghost
over 0.9s on an exponential ease-out. The clock is tied to the PICTURE and
not to a timer — it starts when the first frame lands — so the theatre
never fades into a map that is not there. Verified from the run log rather
than by the eye: the alpha walks 255 → 219 over 93 frames and settles, and
the scrim only ever lifts.

**THE INSTRUMENT.** The card is a MODAL of this console, centred on the
window, over the theatre: square, flat, one 1px `CUT_GREY` edge, a
tracked-caps title on a `PANEL_SLATE` band over a body on `CONSOLE_NIGHT`.
It was docked to the window's top-left corner; the operator called it too
much, and they were right — a door is a door, and a door is in the middle
of the wall it is in. The credential fields are the console's form order —
tracked-caps label over a well, the value in the MACHINE face because a
credential is an id, an empty field carrying the console's dash rather than
a hint word. The submit stays a full-width graphite slab and is SQUARE now,
and it brightens to `MAP_INK` when both credentials are typed: the verb is
ready, and it says so with ink rather than with the accent, because the one
cyan edge on this surface is the field that owns the keyboard.

**THE SCOPE, INSTEAD OF THE READINGS.** The PRE-FLIGHT is gone — a clock, a
frame count, a camera and a picture size are a DIAGNOSTICS panel, and an
operator who has to type a username does not need a stopwatch and a frame
size before the door opens. In its place, under the form, is a **scope**:
the console's own compass face (rim, two rings, twelve graduations, a north
wedge, a needle) drawn in greys, because the one accent on this surface
belongs to the field that owns the keyboard. There is no figure on it and
nothing to read off it: it is decoration, and it is decoration of the
console's own kind — a chart room's door has a scope on it, and the water
the exercise will be fought on is right there under the card. The title
row's far end carries the other ornament, nine hairlines of graded length,
the way a chart's margin is graduated.

**And the surface got smaller in every other way.** The clock is gone, and
with it the sign-in's own repaint drain: the map renders on a gesture, so a
clock driven by the map's frames would have needed its own drain to keep
ticking past the theatre's settling. A surface with no clock has no reason
to ask for frames.

**ONE THING SAID ONCE.** The scope's caption carries the STATE
(`signing in…`) and the status line carries the EVENT only, so the busy line
is suppressed (the caption already says it). The status slot is RESERVED
rather than conditional: a card that grows a line when the server answers
moves the scope under the operator's hands, and decoration that moves is a
bug in the decoration.

## The sign-in, again: a name, a cursor, and one verb (2026-10-20)

Three changes to the same card, and the reason for them is one thing: the
card was carrying a NAME, a CURSOR and a VERB, and it was carrying all three
as afterthoughts.

**THE WORDMARK.** The name is now the largest text on the console and the
only text allowed to be large, set in **Cinzel** — a Roman-capital
inscriptional serif, the letterforms of an engraved plate — tracked at a
sixth of its size, which is the gap a stone-cutter leaves between letters
and the thing that makes an inscription read as an inscription rather than
as a word in a bigger size. Cinzel is a THIRD face, and the type system's
rule is two faces and no third; the rule stands for the console's TEXT, and
this is the one place where the text is a name rather than a reading, so
the face is spent on six letters and on nothing else. The wordmark stands
on the same left edge as the field labels under it, because a name that
hangs over the edge of its own card is a name that was pasted on rather
than engraved.

**THE CARET.** The credential fields are hand-rolled — GPUI 0.2.2 ships no
text field, only the input plumbing — so the console draws the cursor too.
It stands after the value, in the focused field only, in the accent, and it
BLINKS: a steady bar reads as a character the operator typed, and a
blinking one reads as a cursor. The blink runs on the console's own tick
rather than on the system's, so it is the same beat on every machine, and
the caret is lit the moment a field takes the keyboard, because that is the
moment a cursor which happened to be dark would most easily be mistaken for
a field that has stopped accepting input.

**THE SCOPE IS THE VERB.** The plain SIGN IN slab is gone; the scope is the
control, and the operator presses the radar. It says everything the slab
said and more. It BRIGHTENS when the door is armed — both credentials are
there — and it LITS while the round trip is out, and both are TWEENED,
because a radar that jumps from grey to lit reads as a different picture
rather than as the same instrument changing state. Those are two facts, so
they are two channels and not one colour spent twice: `lit` is the link
being out, and `bright` is the door being armed. A console that spends one
colour on two states spends the colour on neither, and an operator who
cannot tell "armed" from "waiting" cannot tell whether the press will do
anything. The caption under the scope is a CAPTION and not a button —
tracked caps in the console's own label style, with no slab and no box —
and it is part of the scope's pressable group, so the two are one control. A
verb nobody can read is a verb nobody will press.

Four new tests (`the_scope_is_the_sign_in_verb`,
`the_scope_says_armed_and_out_as_two_things`, `the_scope_eases_to_its_new_level`,
`the_caret_follows_the_focused_field`); 629 pass, the same four pre-existing
failures.

**`TFG_SIGNIN=busy|refused`** stages the card's own state for a headless
capture, the same seed-the-state precedent as `TFG_SETTINGS`.

Captures: `.impeccable/review/signin-2026-10-20/` — `signin-resting.png`,
`signin-credentials-ready.png`, `signin-busy.png`, `signin-refused.png`,
`signin-signed-in-console.png`. One new test (`theatre_alpha`); the two
readings that the pre-flight had are gone with it.

`cargo test --lib`: 621 pass, the same four pre-existing failures
(`book::prune_drops_relations_for_vanished_drafts`,
`map_render::quad_bow_follows_the_compass`,
`roster::seats_come_before_roles_before_unseats`,
`sim::order_move_applies_then_higher_overrides_loudly`).

**The signed-in console, photographed afterwards.** The hosted API did not
answer during the pass above, so every launch fell back to the fixture
roster and the console never left the card. That was transient. With the
API up, the signed-in console is
`.impeccable/review/signin-2026-10-20/signed-in-console.png`, and the
whole signed-in surface was re-shot — five phases and three modals — with
`scripts/gpui-sweep.sh` running the pixel assertions over each one. The
column's edges, the owned field's cyan perimeter and the one-rule-between-
fields all still hold, so rewriting the signed-OUT surface did not disturb
the console that comes after it. The harness also learned to say out loud
when a capture fell back to the fixture: a picture of the fixture is not a
picture of the deployment, and it used to pass as one.

## The unit details panel: point at a ship, learn what it is

The Game Master's question had no answer in the console. The Inspector dock
existed — a panel of readouts that only a capture could open — and there was
no way to select a hull from the MAP: the markers are painted into a
canvas, so they carry no hitbox, and a press on a ship landed on the map
surface and panned the chart.

**THE CLICK.** A press on the map resolves the hull under it and the release
opens the panel on it. The pick is decided at the PRESS and undone if the
press turns into a pan: GPUI delivers the release to whatever is under the
pointer, so a release that lands on a panel never reaches the map at all —
a release-time pick would be a pick nobody made. The radius is 30 WINDOW
points, converted through the frame's own scale, so a hull is as easy to
point at at one zoom as at another.

**THE PANEL.** Three ladders in one dock: what it is (type, class,
commander, embarkation), where it is (position, course, speed, state,
trail), and the figures the register published for it (length, beam, draft,
displacement, speed max, cruise, range, turn rate). The console never
guesses: a hull with no published specification says so, a hull nobody asked
about says `—` for its embarkation rather than claiming it is on the
water, and a hull with no wire fix says `no fix` rather than drawing a
position.

**WHERE THE FACTS COME FROM.** The register names the hull and its class;
the exercise's piece says who commands it — and, since this pass, what class
the server published for it (the backend used to DROP `unit_class_name`, and
a dropped class is a class the console can only guess at); the wire says
where it is and how fast; the store's spec mirror holds the published
figures, read once at boot — and the figures are asked for again on demand,
below. The embark read names a HOST's passengers, so
the link is read the other way round.

**THE DOCK HOLDS THE POINTER.** The map surface is a full-window hitbox, so
a press on the panel's CENTRE would also reach the map: it would pan the
chart and pick whatever hull was under the button. The dock truncates the
hover path below it, the way the column's islands do.

**ONE ORDER LOOP BUG FIXED ON THE WAY.** The embark-spec read returned from
the order loop when the server could not be reached, which dropped every
LATER order on the floor. A console that stops listening looks exactly like
a console that has nothing to say.

Captures: `.impeccable/review/unit-details-2026-10-20/` — `unit-details.png`
and its crop, against the hosted API. Three new tests: the hit test, the
click end to end through GPUI's own event path, and the drag that starts on
a hull and selects nothing while still panning the chart. `cargo test
--lib`: 622 pass, the same four pre-existing failures.

### The figures were behind the boot, and the type was never on the wire

**The defect.** The panel's published figures and its TYPE both came from
the LOCAL STORE, and the store is synced only at BOOT. An operator who
signed in through the console's own card — rather than with credentials on
the command line — got a dash for every figure and no type at all, until
the next launch. The panel was honest about what it did not hold, and it
held nothing. The class, the commander and the embarkation never had this
problem: they come off the wire.

**The figures are now asked for.** When a unit is selected the shell asks the
binary for that hull's published specification (`ToBinary::ReadSpec
{ unit }`), the binary calls `GET /units/{id}`, and the answer comes back on
its own feed; the panel fills in when it lands. The store's mirror is still
read at boot — a launch with credentials on the command line shows the
figures immediately — and the API's answer is authoritative and overwrites
it. A hull with no published specification still says so: the console does
not invent a figure.

**The type comes off the wire too.** The register's own read (`GET /units`)
publishes each hull's class summary, and that summary carries the class's
TYPE (`unit_class.unit_type.name`). The backend used to drop that field, so
the console could only guess the type from the local taxonomy. The roster
row carries the type name now, and the panel reads it from there when the
drill tree does not know the unit.

**And the embark read is asked for.** The panel asks the binary for the
selected unit's EMBARK READ — the server's per-host passenger list — when
it opens, so "carrying N" is known rather than a dash. The passenger
DIRECTION — a hull that is ABOARD some host — still needs that host's own
read, and stays a dash when the host has not been read. The console says
"on the water" only when it has actually asked.

### And the embark read is per HOST

The embark read answers "who is inside X", so the panel asks for every host
in the exercise — not only for the hull that was pointed at. Reading one
hull tells the console nothing about whether that hull is aboard another
one; reading every host gives both directions, "carrying N" and "aboard X".

**And "on the water" waits for the reads.** A hull that is aboard a carrier
the console never read is not a hull on the water, so the console says a
dash until every host in the composition has been read. A Game Master has no
seat and commands no group, so the commander's own read never runs for one:
the panel's read is the panel's claim, and the claim is the console's to
keep honest.

### What is still owed

- The figures come from the API's CURRENT specification version, so a
  specification version pinned by a game being judged is not what the panel
  shows — and the panel does not say which version it read, because the
  dock has no room for it.


## The after-login screen, rebuilt (2026-10-21)

The operator marked the after-login screen and said the truth about it: two
of the three things on the left had no business being there, and the third
was in the wrong place. The reading was that the screen was cluttered, and
the clutter had a shape — **the empty room was answering questions nobody
had asked.**

### The column is empty when there is no exercise

`view::zone_islands` no longer seeds an OPERATOR island, and `NoSession`
returns nothing at all. The three surfaces that stood there each MOVED:

- **The account** went to the top band's right, as a chip. The island was
  first in every state because identity does not change with the exercise,
  and that reason argues AGAINST a column: a column that changes shape with
  the phase has no business carrying the one fact that does not change with
  it. The chip opens a card with the seat's role and the way out. The band
  has no sign-out button of its own any more — one way to do one thing, in
  one place.
- **The games to hold** became the card in the middle of the frame.
- **The hull register** went to PLANNING, which is the phase that assembles
  a force. It was never a session fact; it read as one beside a session.

The execution rail's ACCOUNT widget went with the account, for the same
reason: a console that says who you are in two places at once has two
answers to the same question.

### The SESSIONS card: two sections, and it opens itself

**THE CARD.** Two columns side by side, one rule between them. The left is
the list of the games this account can see, with the state, the mode and
this seat on each row; the row IS the verb, because a picker with a
separate hold button would be two steps for one decision. The right is the
wizard.

**IT OPENS ITSELF.** Signed in, nothing held, the card is up. A console
that signs in and then waits to be asked is a console that has forgotten the
question it is for. The rule is one rule and it is applied in two places:
`apply_signed_in` (the sign-in answer) and the shell's construction (a
launch that restores a token and holds nothing). A hold closes it.

### The wizard: three steps, and every field the server takes

| step | what it asks for |
|---|---|
| 1 · The exercise | name, mode, pace, description |
| 2 · The window | the real clock's day and its two ends; the exercise clock's two ends |
| 3 · The ground | purpose, target, area, map tag |

**THE WINDOW IS THE POINT, AND IT IS NEW.** The console could not write a
planned window AT ALL: `create_game` sent a name, a mode and five optional
strings, and the gate refuses an exercise whose window is incomplete. The
status file's own "defect that mattered most" section says so — the client
sent neither `POST /games`' window fields nor a `PATCH`, so a session made
through the old form had to be repaired in a second panel before it could
run. The wizard states the window at the door, in the console's own
spelling (a day and two military times, the same fields the Control
island's window editor uses), and `SessionDraft::to_new_game` converts it
to the server's RFC3339.

**THE PACE CAME WITH IT.** `POST /games` accepts `pace`, and `fast` waives
the readiness declaration. The old form had no pace control at all.

**THE ONE FIELD THE WIZARD DOES NOT ASK FOR IS `time_factor`, ON PURPOSE.**
The server accepts it at creation, and the console does not offer it in the
wizard — because the console already owns it, in the Control island, where
it is written through `PATCH /games/{id}/time-factor` and stays live after
the exercise begins. A second control for the same fact is two controls and
one disagreement. The create body leaves the field out, so the server seeds
its own default, and the factor is stated in the place that owns it.

**THE CREATE BODY IS A PURE FUNCTION.** `backend::master::new_game_body`
is the wire shape, and it is tested: the server's own key names, a blank
optional OMITTED rather than sent empty (the server reads an empty string
as "clear this field"), and the window one key at a time.

**THE WIZARD REFUSES WHAT THE SERVER WOULD REFUSE.** A nameless session
does not leave the first step; a half window does not leave the second. The
messages are the console's own (`window::problem`, which both clocks'
rules already lived in).

### The comms door needs an exercise

The COMMS chip was on the band in every state but execution, which put it on
the empty room: an inbox with nothing to receive from. It is gated on a
session being held now.

### Verified

- `cargo test --lib`: 787 pass, the same two pre-existing failures
  (`map_render::tests::quad_bow_follows_the_compass`,
  `sim::tests::order_move_applies_then_higher_overrides_loudly`).
- Captures over the real binary against the hosted API:
  `final-empty-room.png` (the card over the empty map),
  `wizard-window-step.png` (the window step), `identity-card.png` (the chip
  open, with the way out under it), `execution-no-account.png` (the rail
  without the account).
- **The create body against the LIVE API**: `POST /games` with the pace and
  the four window instants answered 201, `GET /games/{id}` read back
  `pace: fast` and all four instants, and the session was then DELETED — a
  game in `planning` can be discarded, and a dev deployment should not keep
  the evidence.
- New tests: the identity chip's disclosure, the card opening itself on
  sign-in, the wizard's steps and its two gates, the draft-to-create-body
  conversion, and the create body's own wire shape.

### What is still owed

- The card's LEFT column is mostly empty for an account with one exercise,
  and the right column's steps are one screen tall. Both are consequences of
  the card being a fixed size, which is the console's rule for every card.

## The window's frame (2026-10-21)

The wizard's window step put the operator's wall time into the request, and
the answer came back in the server's zone. So an operator who typed `0800`
saw `1500` in the panel — and, worse, the panel's answer was about to be
written back.

### What the live API proved

The same experiment, twice, on the hosted dev API. Create a game with a
window, read it back, write back exactly what the read gave, read again:

| | before | after |
|---|---|---|
| the real clock | `08:00Z` → `15:00+07:00` → `22:00+07:00` | `08:00+07:00` → `08:00+07:00` → `08:00+07:00` |
| the exercise clock | `08:00Z` → `15:00+07:00` → `22:00+07:00` | `08:00+07:00` → `08:00+07:00` → `08:00+07:00` |
| **moved by** | **7 hours, every round trip** | **nothing** |

Seven hours, on BOTH clocks, every time the plan was saved. The exercise's
clock is `assumed_start + the scaled elapsed real time` (`models.RunningAt`),
so moving the assumed start moves the exercise.

### The cause, and it is two things

**THE CLIENT READS A WALL TIME AND THE SERVER WRITES AN INSTANT.** The
console's read took the DIGITS after the `T` and threw the offset away; its
write spelled those digits as UTC. Those are the same wall time and two
instants a zone apart, so the read→write pair moved the window by the zone's
offset — every time.

**AND THE COMPARISON WAS A STRING COMPARISON.** `advance::window_update`
decides "did this change?" by comparing the RFC3339 strings. Two spellings
of one instant are one instant, so the string comparison reported a change
the operator never made, and the write that followed moved the exercise
again.

### The fix

- **THE FRAME RIDES ON THE DRAFT.** `WindowDraft` carries the offset the
  server stated, learned from the same strings the digits came from
  (`backend::clock_offset`). A window read back is written back in the frame
  it was read in, and the round trip is the identity.
- **A NEW WINDOW IS WRITTEN IN THE CLIENT'S OWN ZONE.** The typed time is
  the operator's wall time, and the console CAN read its own zone:
  `chrono` is already a dependency and `chrono::Local` is already how
  `src/calendar.rs` decides what day "today" is. So the default frame is the
  machine's own, and the server's frame wins as soon as the server has
  stated one.
- **THE COMPARISON IS AN INSTANT COMPARISON.** `backend::same_instant` reads
  two stamps as instants and compares them. A stamp that does not parse
  falls back to the string compare — the console has no better opinion
  about a value it cannot read.

### The exercise clock's own date

The assumed clock keeps its fixed base date (`2000-01-01`), which is a
fiction the console chose and the server never looks at: the exercise's
clock is that instant plus the scaled elapsed real time, so only the
TIME-OF-DAY is the plan. The base date is the same on both ends, which is
what makes `assumed_end > assumed_start` hold, and that is the whole
guarantee.

### Verified

- `cargo test --lib`: 791 pass, the same two pre-existing failures.
- New tests: the round trip in the server's frame is the identity, two
  spellings of one instant are one instant, the client knows its own zone,
  and an unchanged window is not written even when the spelling differs.
- **Against the live API**, the round trip above: the typed `0800` comes
  back as `0800`, and the instant moves by `0:00:00`. The test sessions
  were deleted.

## The type scale, the titles, and the pickers (2026-10-22)

Three asks in one pass: a font-size setting with a bigger default, titles
that are actually titles, and the two pickers pulled out of the scenario
composer so any form can use them — as popups, under the button that
opened them.

### The titles were the same size as the field keys

`panel_title` drew a surface's name at `LABEL_PT` — 10pt, the same size as
the field key under it, and at the same weight. A title and a field key at
one size is not a hierarchy, it is two kinds of text shouting together, and
the operator is left with no first thing to read.

**A TITLE IS NOW ITS OWN TIER.** `view::TITLE_PT` (16) is three points
over the body, and `panel_title` draws it BOLD where a label is
semibold. Weight is the cheapest hierarchy there is: it costs no width,
and the column this console draws in is narrow enough that width is
scarce. Every surface title moves at once — `SESSIONS`, `SETTINGS`, `NEW
SESSION`, `SCENARIO COMPOSER`, the island bands, the embark card's head.

**AND THE TRACKING FOLLOWS THE SIZE.** Tracking was a fixed 1.2px, which
is 12% of a 10pt label and 7% of a 16pt title — so the two stopped
looking like the same voice at two volumes. `view::label_tracking(size)` is
a ratio now, and every tracked label gets the right air for its size.

### The font-size setting

**ONE MULTIPLIER MOVES THE WHOLE INSTRUMENT.** `view::TEXT_SCALES` is four
named steps (Small, Normal, Large, Largest); `view::pt(base)` is the one
function every size on the console goes through, so a setting is a setting
rather than a set of sizes that can drift apart. The console **boots one
step up** from what it shipped — the operator's word on the shipped size
was that it was too small.

**AND THE CHROME MOVES WITH THE TYPE.** A band that does not grow with its
title is a band the title is cut by, and a column that does not grow with
its dense rows loses a column of them. So the title band, the field
height, the top band and the side column are all read at the scale in
force: `view::title_h()`, `view::field_h()`, `view::top_band_h()`,
`view::zone_w()`. The kernel's geometry reads the same functions the render
does, so the two cannot disagree about where a row is.

**THE SETTING IS REMEMBERED.** `src/settings.rs` is one JSON file under the
data dir, written when the operator changes the size and read before the
first frame — before the layout, because the scale moves the chrome. A
settings file that cannot be read leaves the console at its default size:
a size the operator did not choose is better than a console that will not
open.

### The pickers are components, and they are popups

**`src/pickers.rs` holds the calendar and the time grid**, and they know
nothing about the scenario composer, the session wizard, or the shell. The
composer grew a calendar and a time grid, and the wizard wanted the same
two controls; a second copy of a month grid is a second thing to fix when
the type scale moves.

**THEY OPEN UNDER THE TRIGGER.** A calendar drawn inline pushes the panel
down every time it opens, and a panel that moves under the operator's
cursor is a panel that has lost the field they were in. The popup hangs
under the button, right-aligned with it, and the field does not move.

**AND THE POPUP KNOWS WHERE THE BUTTON IS.** GPUI 0.2.2 has no way to ask
an element where it ended up, so the trigger measures itself: a canvas the
size of the button writes its bounds during the layout pass, and the popup
reads them. The measurements are keyed by the picker, because every
trigger on the surface is measuring itself and the last one laid out is
not the one that is open.

The composer's four window fields are now FOUR targets — a day and a time
at each end — because a day is chosen and an hour is typed, and one
control that answered both was a button that said "calendar" on a row that
wanted an hour.

### The wizard, laid out again

- **THE WINDOW IS TWO GROUPS.** Five labels with no grouping say "five
  things"; two groups say "two clocks, and here is which is which". The
  real clock and the exercise clock each get a marker and one line
  saying what they are.
- **THE MARKERS HAVE AIR ABOVE THEM.** A section marker as close to what
  follows as to what precedes it is a rule, not a heading. A marker with
  room above it is a change of subject.
- **THE FIELDS ARE THE COMPOSER'S FIELDS.** The wizard now uses the same
  row as the scenario composer — a label, a hairline, a trigger, and the
  well — so the two forms are one form twice.

### Verified

- `cargo test --lib`: **804 pass**, the same two pre-existing failures.
- New tests: the title tier is above the body, tracking is a ratio, one
  scale moves the type and the chrome, a scale off the ladder lands on the
  ladder, the console boots bigger than it shipped, a day takes the
  calendar and an hour takes the time grid, every picker has its own
  trigger id, and the wizard spells its times the way the server takes
  them.
- **Captures** in `.impeccable/review/type-scale-2026-10-22/`: the
  settings row with the four sizes, the wizard's window step with the
  date popup open, the time popup, the composer's time popup, the
  sign-in door, and the console's column at the new scale.
- The impeccable layout detector over the changed files: **no findings**.

## The card's padding (2026-10-22)

A screenshot of the SESSIONS card, and one word: *padding*.

### The title was on the card's edge

The card's title band is the only part of that card that is not inside one
of its two columns — so it was the only part that carried no inset of its
own. "SESSIONS" sat on the border and the close cross sat in the corner,
while the two columns below sat a full inset in. The card's name and the
column's name were on different left edges, and the eye read one surface
as three.

**THE BAND CARRIES THE SAME INSET THE COLUMNS CARRY.** The card's name
and the column's name now share one left edge, and the cross is off the
corner.

### The list's rows were on a different edge from the list's own title

The "GAMES" title sat at the column's inset and the rows sat six further
in, so the list's name and the list's items were on two left edges. The
row keeps its trailing inset and gives up its leading one.

### The card could be taller than the window

`SESSIONS_H * text_scale()` was clamped to the window and then floored at
320 — and the floor was applied AFTER the clamp, so on a window shorter
than the floor the card was taller than the window. A card taller than its
window is a card whose title band is cut off by the window's own edge: the
operator loses the name of the surface they are in, and the band exists to
carry that name.

**THE FLOOR IS A WISH, NOT A RIGHT.** The card would LIKE to be at least
240 by 240; it may never be bigger than the window it is opened in.

**AND THE TITLE STATES ITS OWN LINE HEIGHT.** The band is a fixed height
and the title is centred in it, so the title's box has to be the title's
own to state — at a fifth above the size, which sits inside the band at
every rung of the ladder.

### Verified

- `cargo test --lib`: **806 pass**, the same two pre-existing failures.
- New tests: the card fits any window it is opened in (six windows, from
  1280x900 down to 240x200), and the title band holds the title at every
  size on the ladder.
- Capture: `.impeccable/review/type-scale-2026-10-22/sessions-card-padding.png`
  — the card's name and the column's name on one edge, and the rows on the
  same edge as the title.
- The impeccable layout detector over the changed files: **no findings**.

## The planning phase is a GRID (2026-10-22)

The owner's direction: the planning phase is not a column down the side of
the map. It is **six equal islands hovering over the map**, covering the
frame but the top band, each a little bit transparent so the water shows
through, with a margin between each.

### The six, and what is in them

| panel | what it carries |
|---|---|
| **ESSENTIALS** | the session's own terms, AS A FORM: the name, the mode (read back — it is fixed at creation), the pace, the plan's prose and the planned window, editable and written by one Apply; then the facts it reads — the room key, the game clock's multiplier — and, in a scenario exercise, THE BOOK: the scenario's title, its steps, and the door to the composer |
| **EXERCISE** | the mark, the kind of game, the phase, and FINISH PLANNING — and nothing else |
| **PLAYERS** | who is SEATED, by name and role, and the door to the player picker |
| **FLEET** | what fights, with its commander and its placement, and the door to the fleet picker |
| **UNIT** | the selected hull, in full |
| **GROUPS** | the task organisation |

**THE CARDS ARE CARDS.** Rounded corners (`GRID_CARD_RADIUS`), a 16pt
margin between them and at the frame's edge (`GRID_GAP`, `GRID_INSET`),
and a 16pt body inset (`GRID_PAD`). Twelve points of margin over dark
water read as the six panels touching: the gap was there and the eye
could not find it.

**FIVE CARRY A TITLE BAND AND THE EXERCISE DOES NOT.** Its first line is
the wordmark, ARCONS, which is already the name of the surface; a band
above it saying EXERCISE would be a second heading over the loudest thing
in the cell.

**ESSENTIALS IS THE WIZARD'S FORM, AFTER THE WIZARD.** The same questions
the creation wizard asks, asked again of the session that exists: a typo
made at the door is fixed in the panel that shows it. The name, the pace,
the three prose fields and the planned window are drafted and written as
ONE partial edit (`ToBinary::SaveTerms` → `PATCH /games/{id}`), only the
fields that moved (`crate::terms`), because Minos reads an empty string as
"clear this" and a form that restated every field would blank what it
never opened. The MODE is the one field the card does not offer: the
server fixes it at creation.

**PLAYERS AND FLEET ARE LISTS WITH A DOOR.** The account directory, the
role pills, the unseat buttons and the register's ADD rows were three
lists in a cell a third of the window wide. They live in the modals the
column's own cards open (`WhichPanel::Player`, `WhichPanel::Fleet`), and
the cards carry what the session HOLDS plus the one verb that opens the
surface where it is chosen.

**THE CONTROL ISLAND IS GONE.** The multiplier, the hold and the window
are the exercise clock, and the exercise clock is part of the exercise's
terms — so they live in the terms panel now, and the planning room does
not say the exercise's name in one panel and its clock in another.

**THE VERB MOVED TO THE PHASE'S OWN PANEL.** "Finish planning" is not a
term; it is the end of the phase, and it belongs beside the phase's name.

**THE ROLES BELONG TO THE SEAT.** The room's rows carry every role the
game publishes, and an account in the picker is seated by a click that
takes the game's first player-side seat — because a list of accounts with
the whole vocabulary under every name is a wall of pills, and the pills
were running off the panel's edge.

### The grid is the kernel's

`view::planning_grid` computes the six cells; the shell draws them. The
cells are EQUAL (a grid whose cells differ is a grid whose importance is
decided by size), they sit BELOW the top band (the band is the clock and
the identity, and it is the same in every phase), and they carry a margin
(`GRID_GAP`) and an inset (`GRID_INSET`).

**THE COLUMNS FOLLOW THE WINDOW.** Three on a window wide enough to hold
them, two on one that is not, one on a window too narrow even for that:
a cell narrower than the console's own 320pt column cannot hold the
console's own panels, and a grid of cells that cannot hold them is a grid
of cut-off rows.

**AND A PANEL'S BODY IS THE CELL'S WIDTH, STATED.** A body inside a
scroll container has no width of its own: it is as wide as its widest
line, and a long line is as wide as the line is long — so it ran off the
panel's edge and took the rows beside it with it. A definite width is
what makes the text WRAP, and a wrapped line is what makes a narrow cell
readable.

### The cards were crumpled, and the reason was a type

`view::Box` is **corners** `(min_x, min_y, max_x, max_y)`. The renderer drew
each card from `let (x, y, w, h) = cell`, which type-checks and sets the
card's WIDTH to its own right coordinate and its HEIGHT to its bottom
coordinate. Every card therefore ran to the far edge of the window: the six
overlapped into one dark field, the row-two cards were drawn over the bottom
of row one, and the "gaps" the geometry had were covered by the neighbours.
The operator's report was "they look crumpled with each other now", and the
stray border line crossing the chart was one of those mis-placed edges.

The fix is a NAME for the other shape: `view::Rect` is `(left, top, width,
height)` — what an element builder takes — and `view::box_rect` is the one
conversion. `sessions_card_rect` and `sessions_first_row_point` were already
origin-and-size while typed `Box`; they are `Rect` now, so the two shapes
cannot be confused at a call site again. `a_grid_cell_converts_to_the_card_
that_sits_in_it` pins the conversion against the grid the renderer draws.

### A hull in the air clears the grid

A drag out of the fleet picker is a drop on the WATER, and the water is under
all six cards. With the grid drawn, every point a hull could be aimed at
belonged to a card, so the operator could only drop on the strip of chart the
six did not cover.

`IslandDemo::grid_visible` is the one answer — planning AND no drag in
flight — and both the render and `over_grid_panel`, the map's own gesture
guard, read it. The cards stand aside for the gesture and come back on
release, the same rule the picker itself follows.
`a_hull_in_the_air_clears_the_planning_grid` drives it: a point inside the
second cell is the card's while planning and the map's while dragging, and
the cards return when the gesture ends.

### The map is still live under the panels

The grid covers the water, so the map has to know when a gesture belongs
to a panel instead: a press on a panel is the panel's, or the operator
would pan the chart with every click on a verb. **ARMED IS THE
EXCEPTION**: a hull waiting to be placed is placed by clicking the map,
and the map is under the panels — the operator can see it, and they mean
it.

The panels carry no shield for the same reason: a shield would stop the
map's drop target, and a hull could never be placed on the water at all.
A wheel over a panel is the panel's.

### The phase is one answer

`IslandDemo::phase` is where the phase is read, and the column and the
grid both ask it. `TFG_PHASE` seeds it ONCE, at the console's
construction, on the session's own state — so a capture of the planning
room photographs a planning console and not an executing one with a
planning layout laid over it.

### Verified

- `cargo test --lib`: **819 pass**, the same two pre-existing failures.
- New tests: the grid is six equal cells inside the window, the columns
  follow the window, and the planning column is EMPTY (the grid is the
  phase); five cards carry a title band and Exercise does not; a card is
  rounded and stands off its neighbour; the terms draft writes only the
  changed field, refuses a nameless session, and joins the window into
  ONE request (`crate::terms`); and the PATCH that request becomes carries
  only what moved (`backend::master`).
- Captures in `.impeccable/review/planning-cards-2026-10-22/`:
  `planning-cards.png` (the six cards at 768pt wide — two columns), and
  `player-picker-door.png` / `fleet-picker-door.png` (the two doors' modals,
  open over the grid at 1536pt wide — three columns). The gutters show the
  map between the cards, the cards end inside the window, and no edge is
  drawn where a card is not.
- The column's own tests moved to the phase that has a column: the
  closure stack is what overflows, and the crew body is what has room.
- Captures in `.impeccable/review/planning-grid-2026-10-22/`: the grid on
  a wide window, the grid on a narrow one in a scenario exercise (with
  THE BOOK and its steps), the grid with a hull selected, and the
  preparation column unchanged.
- The impeccable layout detector over the changed files: **no findings**.

## The Scenario window, v3: two states, and steps and units as columns

The v3 wireframe asks for ONE surface in TWO states, and this slice is both.
While the map is the work, execution carries the compact widget; opening it
takes the frame. Neither is a card over the chart while the exercise runs.

**The frame.** `board_open && session.execution` is a full-frame branch: an
absolutely positioned element from the top band's height to the bottom of
the window, filled `CONSOLE_NIGHT`, with the map, the execution widgets, the
comms island and the commander's rail behind it. Planning keeps the card it
had: there the map is what the operator is working ON, and the plan is read
beside it. The band stays above either way, because it is the console's own
row and not the surface's.

**Three columns became two.** v2 drew the steps, the step's detail, and an
action column. The action column's only unique content was one line, so it
is a BAR now (`scenario_action_bar`) — the line on the left, and the step in
play's progress on the right — and the width it freed went to the units.
The third column stays removed; it is not coming back as a rail.

**Seven states, seven words.** `StepStatus` is `Idle`, `Played`, `Current`,
`UnitReady`, `Playing`, `Halted`, `Done`, and each has its own ink and its
own lowercase word. The derivation reads the server's own facts rather than
the console's wishes: the pointer from `current-step`; the gate from each
step's readiness read; the clock from the assumed stamp; and `done` from the
step's OWN window end being behind that stamp. `halted` reads this console's
last clock answer, because the clock has no GET — the write is the read. A
pause pressed here turns the step to HALTED and the progress track stops
with it; a pause pressed by somebody else's console is invisible until this
one presses a verb. That limit is the server's, not the surface's, and it is
said here rather than papered over.

**The head is the pointer when the server names none.** A game enters
execution with no `current` and no `asking`, and the window used to read that
as "between steps": every row IDLE, the action bar saying nothing was owed,
and the units column empty — over a book whose own box said "0 of 1 ready,
held by RIZKY HI". The head (the one row the exercise can move from) is now
the pointer while neither of the server's pointers exists: CURRENT while its
gate is held, UNIT READY once every unit has declared. And only then — with
a step in play or a step asking, the head is the NEXT row, and marking it
current too would put two rows in the present tense.

**The units are the tree they sit in.** The third column was removed and the
tree took its place. Rows come from the step's related list, expanded through
the task organisation to each hull's chain of groups, pre-order and
deterministic (sub-formations first, then hulls, each by id). Connectors are
drawn from the parent links, not from a depth number: a rule runs down a
branch that has a sibling after it and elbows into the last child. A group
row says what it holds and nothing else, because a group owns no task and
declares nothing. A hull row says name, commander and ONE chip — READY or
NOT READY — and `NO DECLARATION` when the step does not ask it at all. The
readiness join is the gate's `personnel` against the unit's commander user
id, which is a fact the server publishes.

The gate's personnel are also the SECOND source of units, and that is not
belt-and-braces: this console reads a step's related list for the step the
server names, and a game that has not started names none — so a window that
trusted the related list alone drew an empty column over a step two
commanders had been asked about.

**Progress is the step's own window, filled by the assumed stamp.** Not a
client countdown, not a heartbeat: the server's stamp against the step's
`0700–0900`, read-only, with the clock's own readout beside it. It lives in
the action bar because the bar is the window's one live line, and on the
step's box it would be a second copy that could disagree.

**Verbs stay on the step.** `PAUSE CLOCK` sits inside the playing step's own
box, beside `END` and `REPLAY`, and there is no footer — the design removed
it, and the clock is the step's business. `End Session` does not exist on
this window at all: closing it is the header's `×`, and ending the exercise
is not a thing a scenario window offers.

### Verified

- `cargo test --lib shell_gpui`: **238 pass**, 0 fail. New this slice:
  `a_book_that_has_not_started_stands_on_its_head` (the head is CURRENT, the
  bar names who it waits on, the gate's commander puts the hull in the tree
  with NO related list read), `a_head_whose_gate_is_met_reads_unit_ready`,
  `the_step_states_say_what_each_step_is_doing`,
  `a_step_in_play_inside_its_window_reads_playing`,
  `the_gate_decides_between_current_and_unit_ready`,
  `the_units_tree_hangs_from_the_parent_links`, and
  `every_status_has_a_word_and_a_colour_of_its_own`.
- Captures, over the real binary and the hosted dev API, in
  `.impeccable/review/scenario-window-v3-2026-10-23/`:
  `widget-current.png` (execution beside the map: the compact widget with
  its status, `0/1 ready` and the TO DO line), `current.png` (the window at
  768pt: CURRENT, TO DO — RIZKY HI has not declared, and the tree
  GUGUS ALPHA → UNSUR BRAVO → KRI Abdul Halim), `unit-ready.png` (the same
  step with its gate met: UNIT READY, `1/1 ready`, every chip READY, and the
  bar owing nothing) and `playing.png` (`PLAYING` with the clock's 50% track
  and `08:00 of 0700–0900`, `END` / `REPLAY` / `PAUSE CLOCK` inside the
  step's own box).
- The exercise photographed is a scratch book this slice authored on the
  hosted deployment — game 81, `TGF-V3-TREE`: one scenario, three steps, two
  hulls, and the task organisation `Gugus Alpha > Unsur Bravo / Unsur
  Charlie` — so the capture is of server data, not of a fixture.

### Gaps this slice could not close, and why

- **The per-unit task.** The server publishes no task per unit and no task
  per group: the readiness read names personnel, the related read names
  posts, and neither carries an objective. The window says so in a note
  above the tree rather than inventing one.
- **A second hull under one formation in a photograph.** The book is frozen
  once the exercise runs ("may only be written while the exercise is in
  `planning` or `preparation`"), and a step's related list can name a node,
  which the tree expands — but the step the window was photographing names
  one hull, and re-relating it would have meant writing a running book. The
  expansion is pinned by `the_units_tree_hangs_from_the_parent_links` and by
  the node case in `a_book_that_has_not_started_stands_on_its_head`; the
  picture shows a formation, a hull under it, and an empty sibling formation
  the tree does not draw because it holds none of the step's units.
- **A step this console never watched.** Minos publishes no runs read, so
  `end` takes the pointer away and leaves a book that looks untouched. A
  console that joins after a step has ended reads that step as the HEAD and
  says UNIT READY — which is true of the row, and the reason it is not
  called PLAYED is that this client never saw it play (`joined-late.png`).
  A console that watched it reads PLAYED, and the head moves on.
- **The faint foliage in some captures** is the desktop at Omarchy's default
  window opacity (`0.985 0.96`), not a hole in the window: the band, the
  panels and the fills are opaque in the app, and the same band pixels are
  identical with the board open and closed.
