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