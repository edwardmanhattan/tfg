# The creation flow — implementation handoff

This is the build order for the eighteen frames on the tldraw board
`Game Session.tldraw`, page **`Flow v2 — draft first`**, revision 3 (24 Oct
2026). It is written for an agent that will implement the flow in the GPUI
console, one slice at a time, and it assumes no memory of the conversation
that produced the frames.

Read it with four other documents, in this order:

| Document | Why |
| --- | --- |
| `docs/minos-backend-postman.json` | The API. Authoritative on every write, refusal and ordering constraint. |
| `CONTEXT.md` | The console's domain vocabulary. |
| `docs/agents/verification.md` | The only verification order that is fast enough to use. |
| `docs/gpui-gotchas.md` | Who receives a gesture, before you reason about a drag. |

`docs/minos-api.yaml` predates `pace`, `max_echelon_rank` and
`/scenario-missions`. Where the two disagree, **the postman collection wins**.
`docs/scenario-composer.md` predates `/scenario-missions` too, and its claim
that a saved scenario lives in `scenarios.json` on this machine is **no longer
true**: a Saved Scenario is a server-side detached mission. Where this document
and `scenario-composer.md` disagree, this one wins.

---

## 0. Authority, and the one rule about conflicts

Three things can disagree: the wireframe, the API, and this document.

1. **The wireframe is authoritative on interaction and layout.** What is on
   screen, what a press does, what the operator sees when a thing is refused.
2. **The API is authoritative on what can be written, and in what order.** If
   the frames draw a write the server will not accept, the write changes.
3. **Where they conflict, the conflict is resolved in §12 and nowhere else.**
   Do not resolve it silently in code. Every erratum below is a place where a
   literal implementation of the sheet would fail against the live server.

The sheet is a **light blueprint**: white paper, blue ink, grey for what is
absent. The app is `CONSOLE_NIGHT`. The sheet's palette is drawing convention,
**not a restyle instruction**. §13 maps every sheet colour to an existing
token; no new colour is introduced by this flow.

### Words

| Say | Never say | Why |
| --- | --- | --- |
| **Saved Scenario** (noun), **Save as Scenario** (verb), **Saved Scenarios** (collection) | template, mission, library | The API's row is `scenario_mission`; the operator's word is scenario. "Template" is the CMS's word for the role vocabulary, and using it here makes two different things sound like one. |
| **Game session** / **game** | session, exercise | The list is Game Sessions; the thing a draft becomes is a game. |
| **Static**, **Maneuver**, **Scenario** | mode names in lower case | They are labels on a stamp, not prose. |
| **Not Playing** | unseated, removed | It is a picker value, and it means the participant row is deleted. |
| **In game, unplaced**, **In game, placed**, **Not in game** | selected, ticked, added | The three card states. There is no selection model. |

---

## 1. The shape of the flow

After sign-in there is a **history** and **one question**. Answering the
question opens a **local draft**. The draft is not a game: nothing is written
until one of three verbs is pressed in stage 06.

```
landing ──(mode, + start-from)──▶ draft ──▶ 01 essentials
                                         02 units
                                         03 groups
                                         04 scenario   (scenario mode only)
                                         05 players
                                         06 review ──▶ one of three verbs
```

The three verbs, and where each stops:

| Verb | Writes | Lands in | The draft afterwards |
| --- | --- | --- | --- |
| **Save as Game Session** | game ladder 01–07 | `planning` | Stays open beside it |
| **Save as Scenario** | mission ladder M1–M3 | Saved Scenarios | Closes into the collection |
| **Play Directly** | game ladder 01–07, gate 08, transition 09 | `preparation` | Closes into the game |

A **static** or **maneuver** draft has **five stages and two verbs**: no stage
04, no Save as Scenario (the server refuses a scenario on a non-scenario game
with 409, and a Saved Scenario has no mode to belong to). Stage numbering is per mode:
for those two modes the rail reads `01 02 03 04 05` for essentials, units,
groups, players, review. The **write ladder's** numbers are a different series
and do not move (§9).

The draft is **local until the press**. Every stage edits memory. The map is
live but the draft's pins on it are not the server's placements.

**The rail navigates.** A stage is reached by clicking it, not by finishing the
one before it. There is no auto-advance and no forward lock; the only gated
things are the three verbs, which unlock when the draft is complete enough to
write. A draft half-built is a draft the operator can leave and come back to
within the session.

---

## 2. Frame index

Nine rows, two frames per row. Labels sit 30 px above the row; a frame is
1120 × 700 at x = 80 (left) and x = 1320 (right); row origins are
y = 730 + 860·n.

| Row | Left | Right |
| --- | --- | --- |
| 1 | **F1a** Landing — nothing chosen | **F1b** Landing — scenario chosen |
| 2 | **F2a** Stage 01, top of the form | **F2b** Stage 01, scrolled to the time base |
| 3 | **F3a** Stage 02 — the register at rest | **F3b** The peel — press, move, the UI gets out of the way |
| 4 | **F3c** The drop lands — in the game, on the map, in the basket | **F3d** The basket, open — a saved plan reopened |
| 5 | **F3e** Map mode — moving what is already there | **F4a** Stage 03 — the composition board |
| 6 | **F4b** Settled nesting — and the refusals | **F4c** A group dropped on the map — the ring pack |
| 7 | **F5a** Stage 04 — the composer at rest | **F5b** The basket, nested — a group dropped on a sub-step |
| 8 | **F6a** Stage 05 — people on the left, seats on the right | **F6b** The basket as a drop target — commander |
| 9 | **F7** Stage 06 — the commit rail | **F8** Arrival — what each press leaves behind |

Each frame carries a bottom strip reading `MAP · LIVE TILE LAYER, UNCHANGED`.
That is literal: the map is the console's existing map, in its existing strip,
and this flow does not touch it except to add draft pins and to expand it in
map mode.

---

## 3. The landing, and the draft

### 3.1 The landing (F1a, F1b)

Two columns, one history and one question.

**Left — the history.** `GAME SESSIONS · 08`, five tabs — `ALL`, `PLANNING`,
`PREPARATION`, `EXECUTION`, `CLOSURE`, each with a count — the search
affordance, and the rows:

```
NAME                    MODE      STATE        WHEN
Baltic Shield 26        MANEUVER  PLANNING     23 OCT   ›
North Sea Patrol        SCENARIO  PREPARATION  22 OCT   ›
Harbour Defence         STATIC    EXECUTION    21 OCT   ›
```

- Tabs send `state` to `GET /games?state=`. **The counts come from the
  unfiltered read**, cached: a filtered page cannot count the other tabs.
- **Five tabs, because there are four states** (erratum 14). F1a draws four and
  its own example contradicts itself — a row reads `PREPARATION` while the four
  counts sum to the total without one. `preparation` is a real state and the
  state a Play Directly press lands in; it needs a tab.
- Four columns, not five. The sheet's `ORIGIN` column has no API backing
  (erratum 11); `WHEN` is `created_at`.
- **Single click selects; double-click opens the console.** That is the
  existing behaviour and it does not change: `DOUBLE-CLICK A ROW TO OPEN THE
  CONSOLE`.
- `⌘K SEARCH` is the console's command palette, unchanged by this flow.

**Right — the one question.** `CREATION WIZARD`, `MODE`, and three cards:

| Card | Line |
| --- | --- |
| `01 STATIC` | `Positions set by hand; orders are refused.` |
| `02 MANEUVER` | `The exercise integrates helm orders.` |
| `03 SCENARIO` | `Authored steps, sub-steps, audiences.` |

`Everything else is asked inside the draft.` The mode is the only question the
landing asks, and it is create-only — the draft can show it but never change it.

**Choosing SCENARIO reveals one more question below it: `START FROM`.**

```
BLANK SCENARIO         0 STEPS    No steps. The composer starts empty.
Baltic Shield 25 · Surface   9 STEPS · 18 OCT
North Sea Patrol · Escort   14 STEPS · 15 OCT
Harbour Sweep · Littoral     6 STEPS · 11 OCT
```

Those rows are `GET /scenario-missions` — the server's detached missions, which
is what a Saved Scenario is. `STATIC · MANEUVER HAVE NO SUCH LIST — ALWAYS
BLANK`, because a mission is a scenario and neither of those modes can hold one.

`UNCHANGED BY THE CHOICE: THE HISTORY IS READ, NOT FILTERED BY MODE.` The list on
the left is the games this account can see; picking a mode filters nothing.

**`PROCEED`.** Disabled with nothing chosen (`NOTHING SELECTED · PROCEED IS
DISABLED`). Enabled once a mode is chosen — and, for scenario, once a START FROM
row is chosen, where `BLANK SCENARIO` is a row. What it does:

| Selection | Proceed does |
| --- | --- |
| a mode, no game row | opens a `Blank` draft at stage 01 |
| a `planning` game row | opens a `Reopened` draft seeded from it |
| any other game row | opens the console, as a double-click would — there is nothing left to draft |

**THE LAST ANSWER WINS** (erratum 15). The landing has one verb and two
questions — a mode (create) and a row (open) — and a row left selected from an
earlier look used to retarget that verb: the operator chose a mode, pressed
`PROCEED`, and landed in the selected game's console instead of the draft they
had just described. So each answer withdraws the other: choosing a mode clears
the row selection, and selecting a row clears the mode and the START FROM
answer. A row and a mode are never both answered, and `PROCEED` is always about
the thing the operator picked last.

A row in `preparation`, `execution` or `closure` is not editable: the terms,
the force and the tree are fixed once planning ends (the server refuses the
writes with 409). Say so on the row's hint rather than opening a draft that
cannot commit.

### 3.2 Origins

A draft has exactly one origin, and the origin decides what is prefilled and
what commit will do.

| Origin | Made by | Prefilled from | Commit is |
| --- | --- | --- | --- |
| `Blank { mode }` | landing, mode chosen, start-from blank | nothing | a create: every stage is a write |
| `SavedScenario { mission_id }` | landing, scenario mode, a START FROM row | `GET /scenario-missions/{id}` | a create; the mission's hulls enter **hollow** (no positions), and its steps/sub-steps/audiences enter the composer |
| `Reopened { game_id }` | landing, a `planning` row selected, Proceed | `GET /games/{id}`, participants, hierarchy, units, placements, scenarios | a **diff** against what the server already holds |

F3d's "9 placed, 3 unplaced, 12 in game" is a **Reopened** draft. A Saved
Scenario has no positions at all — F8 says so twice (`NO MAP`, `A SAVED
SCENARIO HAS NO POSITIONS`) — so its units enter stage 02 as *in game, unplaced*,
which is exactly the hollow ring.

### 3.3 What the draft holds

```rust
// src/draft.rs
pub struct Draft {
    pub origin: Origin,
    pub mode: Mode,              // static | maneuver | scenario, fixed
    pub pace: Pace,              // standard | fast
    pub terms: Terms,            // the ten fields of stage 01
    pub ceiling: Option<i64>,    // max_echelon_rank — a rank number, e.g. 40
    pub force: ForceDraft,       // hulls, positions, membership
    pub groups: GroupsDraft,     // nodes, ranks, parents, commanders
    pub book: BookDraft,         // steps, sub-steps, audiences
    pub roster: RosterDraft,     // seats, roles, scopes
    pub journal: Journal,        // what a previous press already wrote
}
```

`force`, `groups`, `book` and `roster` already exist as pure draft modules
(`src/force.rs`, `src/groups/plan.rs`, `src/book.rs`, `src/roster.rs`). Extend
them; do not write a parallel model.

### 3.4 The three states a hull card can show

A hull has four independent facts. The sheet draws the first three as card
states, because they are the ones a card can show:

| State | Sheet | What it means | Where it lives |
| --- | --- | --- | --- |
| Not in game | grey hairline | in the register only | `draft.force` has no entry |
| In game, unplaced | hollow ring | in the draft's force, no position | entry, `start: None` |
| In game, placed | filled pin | in the force, with a position | entry, `start: Some(lat, lon)` |

A fourth fact — **in the task organisation** — is stage 03's, and it is
independent of all three (`docs/adr/0017`). A hull can be placed and in no
group, or in a group and unplaced.

The labels say "in game" because the draft becomes the game. For a `Reopened`
draft they are literally true; for a `Blank` one they are the draft's promise.
Keep the labels: they are the operator's mental model, and one set of words for
one set of states beats two.

### 3.5 The basket

The basket is the draft's force, shown as cards. It is **an overlay, never a
sixth column**, and it exists in stages **02, 03, 04 and 05**.

- **Closed by default.** The header is always visible: `BASKET · <n> UNITS · OPEN ▸`.
- **Resizable** from its bottom-right corner (F3d: `624 × 352`, `DRAG THE
  CORNER TO GROW IT`). Size is remembered for the session, not persisted.
- **It floats over content, and the content continues beneath it.** F3d:
  `THE REGISTER CONTINUES BENEATH THE BASKET · IT IS NOT CLIPPED, ONLY COVERED`.
  Do not shrink the register when the basket opens.
- **Two layouts.** A flat grid in stage 02. Nested by group in stages 03–05
  (F5b): an operation holds gugus holds satuan tugas holds cards; loose hulls
  sit under `UNGROUPED · <n> / LOOSE HULLS`.
- **A card is a drop target** in stages 04 and 05 (a hull's audience, a hull's
  commander) and a **group row is a drop target** in stage 05 (a group's
  commander).
- **A card shows the hull**: its photograph when the archive has one, its NATO
  glyph otherwise. The band above the name is the picture, and the ship does
  not have to be read out of a string.
- **A card says who commands it** once a commander is assigned
  (`BY LT. FITRI`), and `NO COMMANDER` while it is not; a group's channel row
  says `COMMANDED BY …` for the body itself.
- **The chip is a handle too**: drag it to put the closed basket somewhere, and
  a click that does not move still opens it.
- `✕` on a card takes the hull out of the force. In a `Reopened` draft that is
  a delete at commit; in a blank one it is simply not written.

### 3.6 The draft is not saved

F7: `THIS DRAFT LIVES IN THE BROWSER · CLOSING THE TAB LOSES IT`. This is a
desktop console, so read it as **the window**: the draft is in memory, and
closing the window loses it. That is the frame's decision and it stays.

One addition that does not contradict it: closing with a non-empty draft asks
once (`Discard this draft?`). A confirmation is not persistence.

---

## 4. Stage 01 — Essentials (F2a, F2b)

**Header.** Left: `TFG · DRAFT · NO GAME RECORDED`; right:
`09:41Z · LOCAL ONLY · ORGANIZER`. The clock loses `LINK OK` and gains
`LOCAL ONLY`, because nothing in this stage is on the server.

**Rail.** `01 ESSENTIALS · NAMED`, `02 UNITS · 0`, `03 GROUPS · 0`,
`04 SCENARIO · 0`, `05 PLAYERS · 0`, `06 REVIEW · —`. The second line of each
row is a count of the thing, never a literal: `02` is `force.len()`, `03` is
`groups.len()`, `04` is `book.steps.len()`, `05` is `seated / considered`,
`06` is `settled / 9` once a press has begun — positions answered, reads
included (§9.1).

**The form.** Ten fields, in this order, in one scrolling column. Two stamps
sit above them and are not fields.

| # | Field | Rule | API field |
| --- | --- | --- | --- |
| — | **MODE** stamp | create-only; shown, never editable | `mode` |
| — | **PACE** stamp | `STANDARD` / `FAST`; default `STANDARD` | `pace` |
| 1 | NAME | required, ≤ 255 | `name` |
| 2 | DESCRIPTION | optional, ≤ 4000 | `description` |
| 3 | PURPOSE | optional, ≤ 4000 | `purpose` |
| 4 | TARGET | optional, ≤ 4000 | `target` |
| 5 | AREA | optional, ≤ 1000, hidden from participants until execution | `area` |
| 6 | MAP TAG | optional, ≤ 255, hidden until execution | `map_tag` |
| 7 | ACTUAL START | optional, required to execute; a real date and time | `actual_start` |
| 8 | ASSUMED START | optional, required to execute; the fiction's instant | `assumed_start` |
| 9 | TIME FACTOR | optional, required to execute; `× 1.0`, assumed ÷ real, > 0 | `time_factor` |
| 10 | **LENGTH** | optional, required to execute; real hours, > 0 | derives both ends |

The scroll counter in the header reads `N OF 10` and tracks **fields reached by
scrolling**, not fields filled. F2a is `3 OF 10` with five fields already
filled; F2b is `10 OF 10`. Do not turn it into a progress bar of validity —
there is already a rail for that.

**PACE is on the stamp row** (erratum 2). It is not decoration: `fast` waives
the readiness declaration at the execution gate (F8), so an operator who cannot
see it cannot understand why their gate is shaped differently.

**The time base is one group: empty, or complete.** Empty is legal while
planning — the server allows it and the gate refuses execution without it. A
half-set base (a start and no length, a length and no start) is a draft state
the form may hold while the operator types, and REVIEW names it. It is never
sent half-set.

### 4.1 How the time base is derived (erratum 8)

The operator types **two starts, a factor and a length**. The ends are never
typed, which is the frame's decision and the reason there is nothing to keep in
step:

```
actual_end   = actual_start   + length
assumed_end  = assumed_start  + length × factor
elapsed      = length × factor          // assumed time, displayed read-only
```

F2b's `ELAPSED · DERIVED, NOT TYPED from the two starts and the factor` is
therefore true as a *display*, but it is not derivable from those three alone —
something must say how long the exercise is, and no frame has that field. Add
`LENGTH` (real hours). It is the smallest addition that keeps the frame's
actual invariant: **no end is ever typed**.

The alternative — typing both ends, which the existing `NewGame.window` already
supports — was rejected because it is two ways to say one thing, and the two can
disagree.

A factor change moves the **assumed** end and the assumed clock, and never the
real window. When the factor is edited on a `Reopened` draft, send
`time_factor` and `assumed_end` in the same `PATCH`.

**Settled on the live API (25 Oct 2026).** Create accepts client-computed
`actual_end` / `assumed_end`, so the ends are sent. `PATCH /games/{id}` does
**not** recompute them from `time_factor`, so a factor edit must carry the new
`assumed_end` with it — which is the rule above, now confirmed rather than
guessed. There is no case where the window becomes a REVIEW blocker for this
reason.

### 4.2 What the sheet calls the fields

F2a's right-hand `FIELD INVENTORY` panel is a **drawing aid, not a UI panel**.
It exists to state which fields are required, which are hidden and which are
needed for execution. Do not build it. The same information belongs on each
field as a right-aligned hint (`REQUIRED · 255 MAX`, `OPTIONAL · 4000`,
`OPT · HIDDEN`, `OPT · EXEC`), which is what the form already shows.

### 4.3 Footer

`STAGE 01 OF 06 · THE DRAFT IS LOCAL` and `THE VERBS UNLOCK WHEN 01–06 PASS —
NOTHING IS WRITTEN BEFORE THAT`. Both are literal, with one substitution: the
denominator is the **mode's** stage count, so a static or maneuver draft reads
`OF 05` and `01–05 PASS` (§1).

---

## 5. Stage 02 — Units (F3a–F3e)

**Header.** `DRAFT · STAGE 02 UNITS`; right `09:41Z · <n> PLACED · <n> IN
BASKET`. Rail: `02 UNITS · FLEET REGISTER`, right `0 IN GAME / 125`.

### 5.1 The register is a five-column cascade

F3a's columns are a cascade, not tabs:

| Column | Count in the frame | Source |
| --- | --- | --- |
| DOMAIN | 3 drawn / 4 documented | `GET /units/asset-groups` — the asset groups (§18.5) |
| KATEGORI | 10 | `GET /unit-categories?id_asset_group=` |
| TIPE | 26 | `GET /unit-types?id_unit_category=` |
| KELAS | 41 | `GET /unit-classes?id_unit_type=` |
| ASET | 125 | `GET /units?id_unit_class=&id_asset_group=` |

The breadcrumb above the columns is the path through the current selection:
`LAUT ▸ FRIGATE ▸ FRIGAT SERBAGUNA ▸ AHMAD YANI ▸ 6 HULLS`, with a count per
rung. Picking a rung truncates the path below it.

**`DOMAIN` is the asset-group rung**, and the word is the client's own: the
API's description of this endpoint records the breadcrumb as
`Domain > Kategori > Tipe > Kelas > Aset`, and says the group is stored on the
category, one rung above it. The frame's `LAUT / UDARA / DARAT` are
`Aset Laut / Aset Udara / Aset Darat` with the prefix dropped for width.

**The endpoint documents four groups, not three** — the fourth is `Aset
Lainnya`, which collects every category nobody has classified, and it is
returned even when it holds nothing. **Render every group the endpoint returns,
not the three the frame draws**; a zero is a number (erratum 6). This is *not*
`movement_domain`, which is per hull and answers how a thing moves (a submarine
moves `Submerged` and is an `Aset Laut`).

### 5.2 Search

The placeholder is `SEARCH HULL, CLASS, PENNANT …`, and it must be true.

`GET /units?search=` matches **name and hull number only** — not class. So the
search is **local over the synced register**, matching hull name, hull number
and class name, which is what makes the placeholder honest and costs no round
trip per keystroke. The register is a few hundred hulls at most, and the
console already syncs the whole table (`Backend::units` pages at 200).

A search ignores the cascade (it searches the register, not the current class)
and the breadcrumb reads `SEARCH · <term> · <n> MATCH` with a `✕` to clear.

### 5.3 The peel (F3b)

There is **no checkbox and no click-to-add**. A press that does not move is not
a command.

1. **Press** a card. After a small movement threshold the card **lifts**: the
   register dims, a ghost card follows the pointer (`KRI AHMAD YANI / AHMAD
   YANI 351`), and the map becomes the target.
2. The map shows the coordinates under the pointer live: `LAT −3.2841`,
   `LON 108.9214`, and `RELEASE TO PLACE`.
3. **Release** places it (F3c). **Esc** cancels: `ESC CANCELS · THE GHOST
   RETURNS TO ITS SLOT`.

Dropping a hull that is already placed **moves** it. One placement write either
way.

### 5.4 What a drop stages (F3c)

`1 HULL JOINS THE FORCE`, `2 START POSITION SET`, `3 CARD ENTERS THE BASKET`.
The card's third line reads `PLACED · NO COMMANDER YET`, and the frame's
warning is the whole of stage 05's reason to exist:

> `COMMIT WRITES 1 THEN 2` · `A PIECE NEEDS A COMMANDER`

A hull with no commander is a legal **draft** state and an illegal **write**.
The draft must reach REVIEW with a commander for every hull, or the verb names
the hulls that have none. Never invent a placeholder commander: `POST
/games/{id}/units` requires a real participant of that game, not on the judge
side.

### 5.5 The basket open (F3d)

A reopened draft, `12 UNITS · 9 PLACED · 3 UNPLACED`, a flat grid of cards,
`✕` on each, `624 × 352` and resizable. The two legends are the contract:

- `FILLED PIN = ON THE MAP · HOLLOW RING = IN THE GAME, NO POSITION`
- `✕ TAKES THE PIECE OUT OF THE GAME · DRAG A CARD OUT TO PLACE OR MOVE IT`

### 5.6 Map mode (F3e)

The map expands to the window. Header: `MAP MODE`, `<n> PLACED · <n> UNPLACED ·
<n> SELECTED`, `ESC TO RETURN`.

- **The right button moves the camera; the left does not.** A right-drag pans,
  the wheel zooms, and the left button belongs to the things on the chart — a
  press on a mark takes that hull, a click on water clears the selection. The
  first build panned on the left, so every hull gesture also dragged the sea.
- Selecting a pin selects the hull (one at a time).
- `⌫ LIFT POSITION` clears the position and **keeps the hull in the game**.
- Dragging a cluster moves every hull in it: `MOVE ONE HULL → 1 PLACEMENT
  WRITE`; `MOVE A CLUSTER → N WRITES, UNIT-ID ORDER, NO BATCH ROUTE`.
- `A FAILURE PART-WAY LEAVES SOME MOVED AND SOME NOT · THE BASKET DRAWS BOTH`.
  This is the commit's behaviour, not the drag's: nothing is written until 06.
  At commit, a partial placement failure leaves the successful ones written and
  the journal says which.

A "cluster" here is a set of pins that move together — a group's hulls, or a
selection. It is a gesture, not a stored object.

---

## 6. Stage 03 — Groups (F4a–F4c)

**Header.** `DRAFT · STAGE 03 GROUPS`; right `4 GROUPS · 5 IN THEM · 4 FREE`.
Rail `03 GROUPS · TASK ORGANISATION`, right `4 GROUPS · 5 UNITS IN THEM`.

### 6.1 The board

A **tree of bodies** with hull cards as leaves:

```
OPGAB JAYA   OPERASI GABUNGAN · 40   Cdr. Anwar
└ GUS-1      GUGUS · 30               Lt. Fitri
  ├ TG-1     SATUAN TUGAS · 20       HOLDS UNSUR · 10
  │  ├ KRI AHMAD YANI 351   ├ KRI SLAMET RIYADI 352   └ KRI YOS SUDARSO 353
  └ TG-2     SATUAN TUGAS · 20
     ├ KRI DIPONEGORO 365   └ KRI SIGMAS 01 361
```

- **Unsur is the hull** — `UNSUR IS THE HULL — WHAT THE BASKET HOLDS, NOT A
  CONTAINER`. There is no Unsur node. A hull is a leaf card inside a body.
- The `HOLDS UNSUR · 10` badge appears on a body that holds hulls. It is
  derived from what the body holds, never asserted, so it cannot lie.
- The echelon picker on the left offers the `/hierarchy` vocabulary with its
  ranks (`OPERASI GABUNGAN · 40`, `GUGUS · 30`, `SATUAN TUGAS · 20`). `UNSUR` is
  not a rung: a hull is a leaf, not a body that holds anything, so it is not
  offered as a level to build.
- **`CAP max_echelon_rank`** is shown in the header as the ceiling, printed as
  the rank number (`CAP 40`). It is a game field, so it is set here (where it has
  meaning) and sent on create (§12.7). The rank check is numeric: a body's
  `echelon_rank` must be **exactly ten above its parent's** — the immediate upper
  level, never a skipped one — and at or below the cap.

### 6.2 Gestures

| Drop | Result |
| --- | --- |
| a hull card on a body | the hull joins that body, if it is a `SATUAN TUGAS` |
| a hull card on empty canvas | a new body is made there and the hull joins it; the echelon picker opens on it |
| a group row on a body | the group re-parents under it, if it is the level directly above |
| a group row on an echelon rung | under the selected body at that level, or a new body at that level when the tree has none |
| a group row on empty canvas | the group becomes a root |
| a body on a body | re-parent, if the rank allows |

**A body is dragged by its own band**, in stage 03, and the thing in the hand is
the body — not its cards. Its upper level can be a group already drawn on the
canvas, or a **rung** in the echelon column: the canvas only takes a parent that
is already on it, and the rung names the LEVEL, so the gesture works even when
the parent does not exist yet (a body is made at that level and takes the drop).

`DROP ON EMPTY CANVAS → A NEW GROUP · PICK ITS ECHELON` is literal: the drop
creates the node, then the echelon picker opens on it, and until a rank is
picked the node is not a body and cannot hold anything.

### 6.3 The refusals (F4b)

1. **A group inside a hull.** `A GROUP CANNOT SIT INSIDE A HULL — AN UNSUR IS A
   UNIT, NOT A CONTAINER — IT HAS NO BODY TO HOLD ANYTHING`. A hull card is not
   a drop target for a group.
2. **A child that is not at the level directly above its parent.** The rule is
   `child.rank + 10 == parent.rank`, not "strictly below": a `SATUAN TUGAS` (20)
   sits in a `GUGUS` (30) and **not** in an `OPERASI GABUNGAN` (40), even though
   that is above it. `THE BODY DASHES BEFORE THE RELEASE`. Refuse locally, naming
   the level that would take it as well as the one that will not.
3. **A group inside itself, however deep the path.** `THE TREE WOULD STOP BEING
   A TREE`. Refuse locally, naming the path.
4. **A hull on a body that is not a `SATUAN TUGAS`.** A hull IS an `UNSUR`
   (rank 10), so its immediate upper level is the `SATUAN TUGAS` (20); a `GUGUS`
   or an `OPERASI GABUNGAN` holding hulls directly would skip the level the
   organisation says is there. Refuse locally and name the level that holds them.
5. **A node from another game.** `THE SERVER ENFORCES IT THROUGH A COMPOSITE
   KEY`. This one cannot happen in a client that only holds this game's nodes,
   so it is a **rendered server refusal**, not a local check. Do not write a
   local mirror of a rule the client cannot break.

And what is allowed: `A CHILD SITS ONLY UNDER THE LEVEL DIRECTLY ABOVE IT ·
ONE HULL IN ONE PLACE · RE-DROPPING MOVES IT`. One hull in one body: dropping it
on a second body moves it, and the first body loses it.

### 6.4 The ring pack (F4c)

A **group** dropped on the map packs its hulls around the drop point. A hull
dropped on the map is one placement, not a pack.

This is the one piece of geometry in the flow, so it is a pure module
(`src/ring.rs`) with its own tests:

```
anchor   = the drop point
ring k   holds 6k hulls, at radius k·R
packing  = depth-first through the group tree; within a ring, unit-id order
spread   = 360° ÷ N, starting at 12 o'clock, for a partial ring
R        ≈ 0.02° of latitude; longitude scaled by 1 ÷ cos(lat)
writing  = every hull's position flattened, sent one at a time in unit-id order
nothing is stored — reopening shows the hulls, not a cluster
```

**Two orders, and they are not the same order.** *Packing* is geometry: it walks
the group tree depth-first so a sub-group's hulls land together, and sorts within
a ring by unit id so the layout is deterministic. *Writing* is the wire: the
draft records N positions, and the commit flattens them and sends them in
**unit-id order**, because there is no batch route and unit-id order is the only
ordering a resumed commit can reproduce. A pack that walks the tree differently
must not change the write sequence, and a failed write at position k must not
shift the ones after it.

The consequences worth stating, because the sheet states them:

- `5 HULLS · ANCHOR + FOUR AT 90°` — the anchor is hull 1 at the centre; the
  other four sit on the cardinals, so the cluster has a vertical and a
  horizontal axis. A full ring of six spreads at 60°: `A FULL RING IS THE
  SPECIAL CASE — THE SAME RULE, 360° ÷ 6 = 60°`.
- Capacity: `0 AT THE CENTRE, 6 ON R, 12 ON 2R, 18 ON 3R`.
- `SAME RULE AT EVERY DEPTH — A SUB-GROUP PACKED INSIDE A RING KEEPS ITS OWN
  CENTRE`. A gugus packed inside its parent's ring gets its own anchor and
  packs its own children around it, recursively.
- `N WRITES IN UNIT-ID ORDER · NO BATCH ROUTE EXISTS`. The draft records N
  positions; commit writes them one at a time in unit-id order, whatever order
  the pack laid them out in.

Nothing about the cluster survives the commit: the server sees N placements and
no group geometry.

---

## 7. Stage 04 — Scenario (F5a, F5b)

**Header.** `DRAFT · STAGE 04 SCENARIO`; right `5 STEPS · 4 SUB-STEPS IN STEP
03`. Rail `04 SCENARIO · STEPS AND SUB-STEPS`, right `STEP 03 · 4 SUB-STEPS`.
Scenario mode only.

### 7.1 The two panes

**Left: the steps.** An ordered list, each row a kind, a number, a name and a
window:

```
01  BRIEFING   ORDERS · 08:00–08:20
02  TRANSIT    ORDERS · 08:20–09:10
03  SWEEP      ORDERS · 09:30–10:15
04  CONTACT    BEAT   · 10:15–10:40
05  RECOVER    ORDERS · 11:00–11:30
```

`+ STEP` appends, and makes the scenario first when the book has none yet (a
scenario draft opens with an empty book, so a button that only appended a step
answered nothing). The step's KIND is a property of the step, set in its header
(`ORDERS` / `BEAT`), not a second add button: a beat is an orders step that
pauses for an event. `STEPS RUN IN ORDER. WINDOWS MAY OVERLAP.` — the API
agrees: windows may overlap, but a new step's window may not begin before the
previous step's window **ended**.

**Right: the selected step.** Its window, `+ SUB-STEP`, and the sub-steps:

```
01  TAKE STATION XYZ   3 UNITS   AHMAD YANI 351 · SLAMET RIYADI 352 · YOS SUDARSO 353
02  TAKE STATION XYA   2 UNITS   OSWALD SIAHAAN 354 · ISKANDAR MUDA 367
03  HOLD RESERVE       1 UNIT    HANG TUAH 371
NO TASK · IDLE IN THIS STEP      DIPONEGORO 365 · SIGMAS 01 361 · BINTUNI 524
```

- `EVERY SUB-STEP SHARES THIS WINDOW` — sub-steps have no window of their own.
- A sub-step is **one command to one audience**. It is a row, not a card in a
  card: **no boxes-in-boxes**.
- The **idle bucket is derived**, not stored: the force minus the union of the
  step's sub-step audiences. It is read-only in this slice. A hull leaves a
  sub-step by being dropped elsewhere or by `✕` on its card in the sub-step.
- The counter is derived from the list. F5a's header says four sub-steps and
  draws three; a count read from the list cannot disagree with itself.

**A beat carries no sub-steps** — `THE SERVER ANSWERS 409`. The composer refuses
a drop on a beat locally, with the same sentence the server would give: make it
an orders step first. Deleting a sub-step from a beat is allowed by the server;
this composer never creates one.

**An audience is written on a sub-step.** The step-create body on both ladders
happens to show a `related` array, and the step-level route that would write one
answers 409 always. Send `kind`, `content` and the window; then send each
sub-step, then its audience. A `beat` therefore carries no audience at all,
which is what "the kind says the step obliges nobody" means.

### 7.2 The basket, nested (F5b)

Stages 03–05 show the force nested by group, every card carrying its commander
line (`COMMANDED BY LT. FITRI`, `COMMANDED BY —`). **Every body is a channel**,
drawn in the tree's own order and indented by depth — an operation's row, then a
gugus under it, then its satuan tugas, then the cards. A body with no hulls
directly mustered still draws its row, because it is the level its children hang
from; the hulls no body holds are the `UNGROUPED` channel, last. A group row
drags out as **its leaf hulls**:

> `A SATGAS DRAGS OUT AS ITS CARDS · AN OPERATION OR A GUGUS AS EVERY CARD
> BENEATH IT`

### 7.3 Drop resolution

Dropping a group on a sub-step resolves **at drop time**, leaf-first, in unit-id
order:

- `RELEASE → 5 UNITS` — the three already there plus the two in flight.
- `RELEASE ADDS 2 · 5 HULLS, 2 NODES, ONE COMMAND` — the two hull cards join
  **this** command; the two group nodes are shown as provenance, not written.
- A hull already in another sub-step of the **same step** is **moved**, which
  preserves `one sub-step per hull per step`. A hull in another step is
  untouched: a hull can be idle in one step and busy in the next.

---

## 8. Stage 05 — Players (F6a, F6b)

**Header.** `DRAFT · STAGE 05 PLAYERS`; right `4 SEATED · 5 NOT PLAYING`. Rail
`05 PLAYERS · SEATS`, right `4 SEATED · 5 NOT PLAYING`.

### 8.1 Two panes

**Left: `FIND PEOPLE`.** A search over the room's people, each row a name and a
**role dropdown**. The dropdown is the only way to seat somebody here, and it
is a dropdown because the game's role vocabulary can be longer than a row of
buttons. Its options are the game's own roles when the session carries them
(`/games/{id}/roles`), the sheet's three otherwise, plus `NOT PLAYING` last.

**Right: `SEATS`.** One row per seated person, role, and scope:

```
GAME MASTER  Cdr. Anwar      RUNS THE EXERCISE
COMMANDO     Lt. Fitri       COMMANDS GUS-1
COMMANDO     Rina Wijaya     COMMANDS KRI 371
REFEREE      Kpt. Sari       JUDGE SIDE · NO SCOPE
+ 4 MORE
```

- `A SEAT IS A ROLE; A SCOPE IS WHAT IT COMMANDS`.
- `ONE PERSON · ONE ROLE · ONE GAME`.
- `CHANGING A ROLE CLEARS READINESS` — the server does this; the client must
  re-read the roster after a role change rather than patch its own copy.
- `A ROLE IS THIS GAME'S OWN COPY` — the ids come from the game, not from
  `/game-roles` (§12.3).
- The picker **reflects the current seat**. `NOT PLAYING` is the DEFAULT for
  someone who has not been seated — not an official role — and a value once
  chosen: it writes nothing and still counts as a decision (erratum 5).
- **The whole seat row is the drag handle**, not only the name: the carry starts
  anywhere on role, name or scope, and the basket row under the pointer tints
  while a seat is in the air so the target is not a guess. A group has a
  commander as much as a hull does.
- **The search is the people's own**, not the register's: one term used to live
  in both fields, so a hull search survived into this stage and filtered the
  people list with no field on screen (erratum 17).

### 8.2 `Not Playing`

`NOT PLAYING WRITES NOTHING — IT DELETES THE PARTICIPANT ROW`.

The **considered** set is `seated ∪ explicitly Not Playing`, and it is the
denominator the rail shows (`4 / 9`). Someone the operator has not touched is
not considered, and does not count. That is what makes the number a statement
about the operator's decisions rather than about the directory.

### 8.3 Scopes

A scope is a **hull card** or a **group row**. F6b:

> `A ROW AND A CARD ARE TWO TARGETS FOR THE SAME SEAT`
> `A ROW TAKES A GROUP COMMANDER · A CARD TAKES A HULL COMMANDER · THE SAME SEAT
> EITHER WAY`

Rules for this slice:

- One person commands **at most one** scope. Dropping them on a second scope is
  refused locally, naming the scope they already hold. There is no hand-over
  dialog here.
- One scope has **at most one** commander. Dropping onto an occupied scope is
  refused, naming the holder. `✕` clears a **group's** commander (a real write:
  `DELETE /games/{id}/hierarchy/{node}/commander`). A **hull's** commander
  cannot be cleared — `PATCH /games/{id}/units/{unit_id}` requires
  `id_commander` — so a hull's scope can only be changed, never emptied.
- A judge-side role holds **no scope**: `JUDGE SIDE · NO SCOPE`. Refuse the drop
  rather than letting the server refuse the write.
- The dragged row **stays in the list until the drop** (`SHE IS LIFTED FROM THIS
  LIST — THE ROW STAYS UNTIL THE DROP`); Esc returns it.

### 8.4 Readiness is not here

`READINESS IS SELF-SERVICE · EVERYONE MARKS THEMSELVES READY; THERE IS NO
SET-READY FOR SOMEBODY ELSE.` That is an explanation in this stage, not a
control (erratum 4). A readiness declaration is only accepted while the game is
in `preparation`, and this stage runs before the game exists.

What the stage does own is the **reason** the gate will be shaped the way it is:
`PREPARATION → EXECUTION NEEDS ALL OF THEM, UNLESS PACE IS FAST`.

---

## 9. Stage 06 — Review and commit (F7)

**Header.** `DRAFT · STAGE 06 REVIEW`; right `09:41Z · <VERB> · WRITE <n>
<STATE>`. Rail `06 REVIEW · COMMIT`, right `<n> OF 9 SETTLED` — the sheet
prints `WRITTEN`, and 08 is a read (§9.1).

### 9.1 The screen

**Left — `WHAT YOU PRESS`.** Three verb blocks (two for static/maneuver), each
with what it writes, where it lands, and a one-line consequence:

| Verb | Line | Consequence |
| --- | --- | --- |
| Save as Game Session | `WRITES 01–07 · STOPS BEFORE READINESS` | `LANDS IN PLANNING`, `IT KEEPS THE DRAFT OPEN BESIDE IT` |
| Save as Scenario | `WRITES ITS UNITS AND ITS STEPS` | `NO MAP, NO GROUPS, NO SEATS`, `LANDS IN SAVED SCENARIOS` |
| Play Directly | `WRITES 01–09 · THE WHOLE LADDER` | `IT DOES NOT START THE EXERCISE`, `LANDS IN PREPARATION` |

A verb that cannot run is **disabled and says why**, using the same plan that
would run it (§9.3): the blockers are the plan's gaps, rendered as sentences.

**Right — `THE WRITE ORDER`.** One row per ladder position: number, name,
one-line description, status. The statuses are `WRITTEN`, `FAILED 502`,
`PENDING`, and `READ` for 08 — the gate is a read, so it is never "written",
and a counter that said it was would be lying about the one position whose
whole point is that nobody writes it. The counter (`6 OF 9`) counts positions
**settled**, read included. `ONE AT A TIME` — the ladder is issued one write at
a time, and the row's status changes as each answer lands.

The numbers on screen are the corrected ladder's (§9.2), not the sheet's. F7's
example failure is on `SEATS`, which the sheet numbers 06 and the corrected
order numbers 02; its statuses are illustrative, the order is not.

**The failure block.** When a write fails:

```
WRITE 02 FAILED · 502 · THE SERVER DID NOT ANSWER
THE FIRST ONE IS ON THE SERVER. NOTHING IS ROLLED BACK, AND THE DRAFT IS STILL HERE.
PRESS AGAIN AND IT RESUMES AT 02 — 01 IS NOT RE-SENT.
                                              RESUME AT 02 ▸
```

Every clause is a rule:

- **Nothing rolls back.** There is no compensation and no delete-the-game.
- **The draft stays.** It is the only record of what was intended.
- **Resume re-plans**, it does not replay (§9.3).
- **A 502 has no envelope** (`data: null`). Show the status and the
  `X-Request-ID` header — the API asks to be quoted that when reporting a
  failure — and never invent a field-level error for it.

### 9.2 The two ladders

The sheet prints one ladder of nine and says all three verbs share it. Both
halves of that are wrong, and the corrections are erratum 1:

**The game ladder** (Save as Game Session, Play Directly):

| # | Write | Endpoint | Why here |
| --- | --- | --- | --- |
| 01 | GAME | `POST /games` | Lands in `planning`. Everything else hangs off its id. |
| 02 | SEATS | `POST /games/{id}/participants` | **Before units**: a hull's commander must already be a participant. |
| 03 | HIERARCHY | `POST /games/{id}/hierarchy`, top-down | A parent before its child. |
| 04 | UNITS | `POST /games/{id}/units`, then `PUT /games/{id}/units/{unit_id}/hierarchy` | Needs the seat (02) and the node (03); the tree placement is a second write per hull. |
| 05 | PLACEMENTS | `PUT /games/{id}/units/{unit_id}/placement` | One write per hull, unit-id order. |
| 06 | STEPS | scenarios, steps, sub-steps, audiences | Audiences name hulls, so after 04. |
| 07 | COMMANDERS | `PUT .../hierarchy/{node}/commander`, `PATCH .../units/{unit_id}` | Group commanders, and a hull's commander where 04's differs. |
| 08 | READINESS | **`GET /games/{id}/readiness`** — a read | The gate, shown before 09. Never a write. |
| 09 | TRANSITION | `POST /games/{id}/transitions {to:"preparation"}` | Play Directly only. |

**The mission ladder** (Save as Scenario):

| # | Write | Endpoint |
| --- | --- | --- |
| M1 | MISSION | `POST /scenario-missions` (title, description, code?) |
| M2 | STEPS | `POST /scenario-missions/{id}/steps`, in order |
| M3 | COMMANDS + AUDIENCES | `POST .../steps/{step}/substeps`, then `PUT .../substeps/{substep}/related {units:[register ids]}` |

The mission ladder writes **no game, no seats, no hierarchy, no placements, no
commanders, no transition**. That is what F8's `NO MAP · NO SEATS, NO GROUPS`
means.

### 9.3 Commit is a diff with a journal, not a replay

Three rules, and they are the difference between a commit that converges and one
that duplicates.

1. **Plan against live state.** `commit::plan(&draft, &live)` returns the writes
   still owed, in ladder order. For a `Reopened` draft that is a diff: a hull
   already in the game is not assigned again; a placement that matches is not
   re-sent; a participant whose role matches is not re-seated.
2. **Record what landed.** Every successful write appends to the in-memory
   `Journal` with the id the server returned. Resume reads the journal, not the
   draft's optimism.
3. **On failure, re-read the family and re-plan.** Re-read the resource family
   that failed (and anything after it that could have changed), then plan again.
   A write the journal says succeeded is never re-sent.

**`POST /games` has no idempotency key.** If 01's response is lost — a timeout,
a 502, a closed socket — the game may exist anyway. Do **not** offer a bare
"retry 01". Re-read `GET /games?search=<draft name>` and look for a game created
in the last few minutes:

- exactly one match → adopt it, journal 01, continue at 02;
- none → offer retry;
- more than one → stop and show them. Guessing here creates a duplicate game
  the operator cannot see.

**Never send `id_mission` on a game commit.** A draft seeded from a Saved
Scenario may have been edited; `POST /games/{id}/scenarios {id_mission}` would
replay the mission and discard the edits. Author the steps, sub-steps and
audiences explicitly. The cost is real — a five-step, four-sub-step plan is
about fourteen writes — and it is the price of the draft being editable.

### 9.4 The draft is local, the press is the only write

F7's footer, literal: `THIS DRAFT LIVES IN THE BROWSER · CLOSING THE TAB LOSES
IT · THE PRESS IS THE ONLY WRITE`. Nothing in stages 01–05 calls a write
endpoint. Reads are free and constant; writes happen only here.

---

## 10. Arrival (F8)

Three presses, three states, one gate. The map does not change between the
three; the state does. A Saved Scenario is the only arrival with no map at all.

| | Save as Game Session | Save as Scenario | Play Directly |
| --- | --- | --- | --- |
| Lands in | `PLANNING · NOT STARTED` | `SAVED SCENARIOS · 1 NEW` | `PREPARATION · LIVE` |
| Force | `9 OF 9 PLACED` | `9 UNITS IN THE BASKET` | `THE SAME NINE PINS` |
| Plan | `9 UNITS · 5 STEPS · 4 SEATED` | `5 STEPS · 4 SUB-STEPS` | `4 SEATED · 0 OF 4 READY` |
| Next | `OPENS AT 01 ESSENTIALS` | `OPEN IT TO START A NEW GAME` | `THE GAME MASTER PRESSES START` |

The `READY` numbers in the table and in the strip below are the **frame's own
values**, and both count the Referee, whom the gate exempts. A built panel counts
the exercise side (erratum 10); the frames are left as drawn so the discrepancy
stays visible instead of being quietly corrected in one place.

**The gate.** `PREPARATION → EXECUTION · THE ONE GATE`. Five conditions, and
the panel shows them as a strip:

```
WINDOW COMPLETE ✓   FORCE 9 UNITS ✓   PLACED 9 OF 9 ✓   COMMANDER GUS-1 ✓   READY 2 OF 4 ✕
                                                                        REFUSED · 409
```

Rules for the strip:

- **The verdict is `can_execute`** from `GET /games/{id}/readiness`. The strip's
  ✓/✕ are the client's own reading, for legibility; if they disagree with the
  server, the server wins and the panel says so.
- **The blockers are the server's sentences, verbatim**, under the strip. The
  client does not reword them (`GameReadinessView` already carries this rule).
- The one place the client adds anything is the readiness sentence:
  `GameReadinessView::names_readiness` answers whether the server's list names
  that condition, and only then may the client attach the outstanding
  participants' names under it. The sentence itself stays the server's.
- The API names **every** missing ingredient at once; the sheet's "the refusal
  names the one thing still missing" is wrong, and the panel must render a list
  (erratum 9).
- `READY n OF m` counts the **exercise side**: `m` is seated participants with
  `is_judge_side == false`, `n` those with `is_ready`. F8's `2 OF 4` counts the
  Referee, who is exempt (erratum 10).
- `fast` waives readiness **and nothing else**: the row reads `WAIVED · PACE
  FAST`, and the other four conditions still decide the verdict.

---

## 11. The API contract

The envelope is always `{status_code, message, data, metadata}`. A 4xx carries
field errors under `data.errors`, keyed by the request field name; a 5xx has
`data: null` and an `X-Request-ID` header. Render `data.errors` under its
fields, and a 5xx as one line with the status and the request id.

### Reads

| Call | Used by | Gives |
| --- | --- | --- |
| `GET /games?state=` | landing tabs | `id, name, mode, state, created_at, updated_at` |
| `GET /scenario-missions` | landing START FROM | saved scenarios with step counts and dates |
| `GET /scenario-missions/{id}` | a SavedScenario draft | title, description, steps, sub-steps, related hulls |
| `GET /units/asset-groups` | stage 02, first rung | asset groups with hull counts; `Aset Lainnya` documented to appear even at zero (§18.5) |
| `GET /helpers` | stage 02 | statuses and the other lookups |
| `GET /unit-categories`, `/unit-types?id_unit_category=`, `/unit-classes?id_unit_type=` | stage 02 cascade | the three middle rungs |
| `GET /units` | stage 02 | the register; `search` matches name and hull number only |
| `GET /hierarchy` | stage 01/03 | echelon levels with `echelon_rank` |
| `GET /game-roles` | stage 05, before the game exists | the template vocabulary: names and `is_judge_side` |
| `GET /users?search=` | stage 05 | the directory; needs `read /system/users` (403 is a real answer) |
| `GET /games/{id}` | Reopened | terms, mode, pace, state, window, `time_factor` |
| `GET /games/{id}/roles` | Reopened, commit | the **game's own** role ids |
| `GET /games/{id}/participants` | Reopened | seats, `is_judge_side`, `is_ready` |
| `GET /games/{id}/units` | Reopened | hulls with `id_commander`, `id_hierarchy_node` |
| `GET /games/{id}/hierarchy` | Reopened | the tree |
| `GET /games/{id}/placements` | Reopened | positions and the placed count |
| `GET /games/{id}/scenarios`, `.../steps` | Reopened | the book |
| `GET /games/{id}/readiness` | 08 | `can_execute`, `blockers`, `fast` |

### Writes

| Ladder | Call | Body | Refusals that matter |
| --- | --- | --- | --- |
| 01 | `POST /games` | `name, mode, pace, description, purpose, target, area, map_tag, max_echelon_rank` (a rank number, e.g. `40`), `actual_start, assumed_start, actual_end, assumed_end, time_factor` | 400 per field; `mode` is create-only |
| 02 | `POST /games/{id}/participants` | `id_user, id_game_role, call_sign` | 409 already seated; 409 role id not this game's |
| 02 | `PATCH /games/{id}/participants/{user_id}` | `id_game_role` | clears readiness |
| 02 | `DELETE /games/{id}/participants/{user_id}` | — | hard delete; idempotent |
| 03 | `POST /games/{id}/hierarchy` | `id_unit_hierarchy, name, parent_id?, icon?, note?` | 400 `id_unit_hierarchy` (rank, ceiling); 400 `parent_id` |
| 03 | `PUT /games/{id}/hierarchy/{node}/parent` | `parent_id` | re-parent |
| 03 | `DELETE /games/{id}/hierarchy/{node}` | — | node and its subtree |
| 04 | `POST /games/{id}/units` | `id_unit, id_commander, note?` | 400 `id_commander` (not a participant, or judge-side) |
| 04 | `PATCH /games/{id}/units/{unit_id}` | `id_commander, note?` | 400 `id_commander` |
| 04 | `PUT /games/{id}/units/{unit_id}/hierarchy` | `id_hierarchy_node` | 400 if the node is not this game's |
| 05 | `PUT /games/{id}/units/{unit_id}/placement` | `latitude, longitude` | both required, may be 0; ±90 / ±180; 409 once play began |
| 06 | `POST /games/{id}/scenarios` | `code?` or nothing | 409 unless mode is `scenario` |
| 06 | `POST .../scenarios/{sid}/steps` | `kind, content, start_hour?, end_hour?` | 400 half-window, `HHMM` strict, end ≤ start, order vs windows |
| 06 | `POST .../steps/{stid}/substeps` | `content` | 409 on a `beat` |
| 06 | `PUT .../substeps/{subid}/related` | `{related:[{kind:"hull", id}]}` | whole list, not a delta; empty clears |
| 07 | `PUT /games/{id}/hierarchy/{node}/commander` | `id_commander` | participant, not judge-side |
| 07 | `DELETE /games/{id}/hierarchy/{node}/commander` | — | clears |
| 08 | `GET /games/{id}/readiness` | — | read only |
| 09 | `POST /games/{id}/transitions` | `{to:"preparation"}` | 403 not this game's GM; 409 wrong step; 409 preconditions, all named |
| M1 | `POST /scenario-missions` | `title, code?, description?` | code normalised to upper case, unique among live |
| M2 | `POST /scenario-missions/{id}/steps` | `kind, content, start_hour?, end_hour?` | as above |
| M3 | `POST .../steps/{stid}/substeps` | `content` | 409 on a beat |
| M3 | `PUT .../substeps/{subid}/related` | `{units:[register unit ids]}` | **register** ids, not game ids |

Two asymmetries to keep straight:

- **Template audiences are `{units:[…]}` of register hull ids; game audiences
  are `{related:[{kind:"hull", id}]}` of game unit ids.** The client writes hulls
  only, never groups.
- **A step-level `PUT .../related` always answers 409**, on both ladders. It
  exists to tell an old client why. Never call it — and never send the `related`
  array that the step-create bodies happen to show. An audience is written on a
  sub-step and nowhere else.

---

## 12. Errata — where the sheet and the API disagree

Each of these is a place where the frames, taken literally, do not build. The
resolution is the one to implement.

**1. The write order.** F7 prints game → hierarchy → units → placements → steps
→ seats → commanders → readiness → transition. `POST /games/{id}/units`
requires `id_commander`, and that commander must already be a participant, so
**seats precede units**; and a hull's tree placement needs its node, so
**hierarchy precedes units**. Use §9.2's order. F7's own footer — "the ladder is
the same order for all three verbs, each verb just stops earlier" — is also
wrong: Save as Scenario has its own, shorter ladder.

**2. Pace is missing from every frame.** `pace` is on create, is editable while
planning, and `fast` waives exactly one gate condition. Add it to stage 01's
stamp row, default `standard`.

**3. Roles cannot be read before the game exists.** `GET /games/{id}/roles`
needs an id, and a blank draft has none. The draft stores role **names** from
`GET /game-roles`; after `POST /games` it reads the game's own roles and maps
name → id before writing seats. Never send a template role id: the server
answers 409 because the two vocabularies' ids do not overlap.

**4. Readiness is not a write in this flow.** 08 is a read. Declarations are
self-service and only accepted in `preparation`. "Play Directly writes 01–09"
means 01–07 plus the gate read and 09.

**5. The role picker must reflect the seat.** F6a shows `Kpt. Sari` as
`NOT PLAYING` while the seats column seats her as `REFEREE`. The picker's value
is the current seat; `NOT PLAYING` is only for someone not considered.

**6. The register's first rung is the asset group, and it has more values than
the frame draws.** F3a draws three (`LAUT`, `UDARA`, `DARAT`) and the endpoint
documents four, including `Aset Lainnya`, which collects every unclassified
category and is returned even when empty. Render the list the endpoint returns;
a zero is a number. Separately: the search placeholder promises class, and the
server's `search` does not match class — so search locally (§5.2).

**7. `max_echelon_rank` is a rank number, not a hierarchy id.** The sheet writes
`OPERASI GABUNGAN · 40`, and that is literal: send `40`. **Driven against the
live API on 25 Oct 2026** — it takes the number, not `id_unit_hierarchy`, so
do not resolve the level to an id. The draft holds the rank (`Option<i64>`), the
stage 03 header prints `CAP 40`, and the local rank check compares numbers:
a child's `echelon_rank` must be below its parent's and at or below the cap.
The ceiling is a game field, so it is sent on create and shown in stage 03's
header where it has meaning.

**8. There is no way to derive the window from two starts and a factor.** The
sheet says the ends are derived and there is no end field; that is right, and it
is one input short. Add `LENGTH` in real hours and derive both ends (§4.1).
Never expose an end as an editable field. Create accepts the client-computed
ends (verified 25 Oct 2026).

**9. The gate names every missing thing, not one.** F8 says "the refusal names
the one thing still missing". The server returns all of them under
`data.errors`; render the list.

**10. F8's readiness denominator counts the Referee.** `READY 2 OF 4` includes
a judge-side seat, which the gate exempts. Count the exercise side.

**11. The game list has no origin column.** F1a draws `ORIGIN` with values
`AUTHORED`, `REPLAY`, `DUPLICATE`; `GameSummary` is `id, name, mode, state,
created_at, updated_at` and nothing records provenance. **Drop the column in
this slice** and render four columns. If provenance matters, it is a backend
field request (`origin` / `source_game_id` on the summary), not a client guess
from names. `WHEN` is `created_at`.

**12. `overlay_text` exists and no frame shows it.** The create body accepts it.
Leave it out of the draft in this slice; it is optional, and a field with no
place on the sheet is a field the operator cannot find again.

**13. The draft lives in a window, not a browser.** Keep the decision (no
persistence), fix the word.

**14. The landing's tabs are missing `preparation`.** F1a draws `ALL`,
`PLANNING`, `EXECUTION`, `CLOSURE` — four tabs for four states, with one state
short. Its own example is the tell: a row reads `PREPARATION` while the counts
sum to `08` without it. `GameState` is `planning, preparation, execution,
closure`, and `preparation` is where Play Directly lands, so it is the tab an
operator will look for first. Render five (§3.1). The counts in the
frame are illustrative; a real tab bar derives every one of them.

**15. The landing's one verb cannot serve two answers.** `PROCEED` sits under
the wizard, but it also acted on a selected history row, and the row won. A row
clicked once (a select, not an open) therefore stayed armed: every mode chosen
afterwards was ignored, and the press opened that game's console instead of the
draft the operator had just described. **The last answer wins**: choosing a mode
clears the row selection, selecting a row clears the mode and the START FROM
answer. A row and a mode are never both answered (§3.1).

**16. Stage 03's tree rules, and its basket, were drawn but not enforced.** Five
defects in one screen, each now a rule with a test:

- **The basket was a flat grid.** It printed the group rows as captions and put
  every card in one wrap beneath them, so a card's channel was a label rather
  than a place. It now draws the same tree the board does: a channel per body,
  indented by depth, the cards under the body that musters them, `UNGROUPED`
  last (§7.2).
- **A rank could skip a level.** `add_group`/`stage_group`/`set_parent` accepted
  any lower rank, so an `UNSUR` could sit straight in a `GUGUS`. The rule is
  `child.rank + 10 == parent.rank` (§6.3.2), and a hull — an `UNSUR` — therefore
  sits only in a `SATUAN TUGAS` (§6.3.4).
- **`UNSUR` was offered as an echelon.** It is the hull, not a container, so the
  rung is gone from the column (§6.1).
- **A group could not be dragged.** The board's band selects but did not carry;
  the only draggable thing was a basket card, which carries hulls. A band now
  carries the BODY, to a body on the canvas or to a rung (§6.2).
- **Dragging the basket panned the chart and stuttered.** The press reached the
  map ground behind the basket, so every move moved the camera; and the drag's
  own move handler lived on the band the drag moves, so GPUI dropped moves whose
  pointer had outrun the panel, while every move re-dirtied the whole window on a
  software rasteriser. The basket now blocks the mouse at its wrapper (so the
  ground never sees the press) and a full-window capture layer owns the drag
  while the grip is held; a move notifies only when the clamped corner actually
  changes.

**17. Stage 02–05's cards, gestures and fields.** Six things the sheet draws
that the build had not made true:

- **A card is a picture as well as a name.** The register's cards, the basket's,
  the board's leaf cards and a sub-step's audience now carry the hull's
  photograph, or its NATO glyph when the archive has none (§3.5).
- **`+ STEP` did nothing on an empty book.** A scenario draft opens with no
  staged scenario, and the button silently returned. It now makes the scenario
  (named for the exercise) and the step (§7.1).
- **`+ BEAT` was a second verb for a property.** The step's kind is set on the
  step (`ORDERS` / `BEAT` in its header), and a beat still refuses sub-steps
  with the server's sentence.
- **The role buttons could not hold a real vocabulary.** The picker is a
  dropdown, fed by the session's roles when it has them; `NOT PLAYING` is the
  default, not a server role (§8.1).
- **A commander was invisible.** The basket's cards say `BY <NAME>` and its
  group rows say `COMMANDED BY <NAME>`; a target tints under the pointer while
  a seat is carried (§8.1).
- **The search bled between stages.** The register and the people list shared
  one term; they now have one field each (§8.1).
- **The left button panned the map.** The right button pans, the left takes the
  marks (§5.6).

---

## 13. Visual rules

The console is the existing console. This flow adds screens, not a style.

| Sheet | App token | Use |
| --- | --- | --- |
| blue ink | `RADAR_CYAN` | active, owned, the thing you are pressing |
| grey ink | `CUT_GREY` | neutral, absent, a disabled verb |
| pale rule | `HAIRLINE_SLATE` | the 1 px rules between rows and columns |
| paper | `CONSOLE_NIGHT` | the field |
| panel | `PANEL_SLATE` | a card, a well's surround |
| well | `DEEP_WELL` | an input, the map's backing |
| white ink | `MAP_INK` / `BODY_SILVER` | names and prose |
| green | `SIGNAL_GREEN` | written, ✓ |
| sand | `WARNING_SAND` | pending, a gate not yet met |
| red | `FAULT_RED` | failed, ✕ |
| rank 10 / 20 / 30 / 40 | `RANK_UNSUR`, `RANK_SATUAN_TUGAS`, `RANK_GUGUS`, `RANK_OPERASI_GABUNGAN` | echelon chips and the tree's edges |

- **Type**: `FACE_MACHINE` (PT Mono) for labels, counts, coordinates, times,
  hull numbers and every status chip; `FACE_HUMAN` (Saira Condensed) for prose
  and people's names. `FACE_MARK` (Cinzel) is the island's, and does not appear
  in this flow.
- **Labels are upper case**; prose is sentence case. A label never ends in a
  full stop; a sentence always does.
- **Counts are derived.** Every number on screen is read from the collection it
  counts. A literal in a frame is an example, never a fixture.
- **The wireframe's boxes are not components.** A frame's ruled box is a
  region; build it from the console's existing row, well and card patterns.
- **No new colour.** If a state seems to need one, it needs a different
  existing ink and a sentence.

---

## 14. Do not

- **No checkbox, no multi-select, no click-to-add.** Press, drag, release.
- **No Saved Scenarios tab on the landing.** The landing's tabs are game states.
  A saved scenario is a row in the landing's `START FROM`.
- **No auto-advance.** Stages are reached by the rail, not by finishing the one
  before. Only the three verbs are gated.
- **No sixth column.** The basket floats, or it does not exist.
- **No boxes-in-boxes** for sub-steps. A sub-step is a row; its audience is a
  list of cards on that row.
- **No pin for a group.** A group has no map marker and no stored anchor. The
  ring pack is the only thing a group does to the map.
- **No step-level `/related` write.** It always 409s.
- **No sub-step on a beat.** Locally refused with the server's sentence.
- **No set-ready for somebody else.**
- **No rollback, no compensating delete, no "undo the commit".**
- **No `id_mission` on a game commit.**
- **No `library.rs`** for Saved Scenarios. That file is the old client-side
  catalogue and is not this flow's store.
- **No restyle.** The sheet's paper is not the app's field.
- **No "template" in UI copy**, and no "mission".

---

## 15. Work slices

Each slice ends in a state that can be verified on its own. The pure modules
come first because they are the only part that can be tested in six seconds.

| # | Slice | Files | Proves itself by |
| --- | --- | --- | --- |
| 1 | **The draft model** | `src/draft.rs` (new) | unit tests: origin, completeness, the derived time base, the considered set |
| 2 | **The ring pack** | `src/ring.rs` (new) | unit tests: capacity, spread from 12 o'clock, longitude scaling, packing depth-first, writing in unit-id order |
| 3 | **The commit engine** | `src/commit.rs` (new) | unit tests with a fake transport: two ladders, diff vs live, journal, resume after a failure at each position, the lost-01 case |
| 4 | **Backend gaps** | `src/backend/master.rs` | scenario-missions (list, get, create, steps, sub-steps, template audiences), game sub-steps, `max_echelon_rank` on the detail read |
| 5 | **Landing** | `src/shell_gpui.rs` | F1a, F1b captures; games read, tabs filter, mode, START FROM, Proceed |
| 6 | **Stage 01** | `src/terms.rs`, shell | F2a, F2b; the ten fields, the stamps, the scroll counter, the derived window |
| 7 | **Stage 02** | shell, `src/dnd.rs` | F3a–F3e; cascade, search, three states, the peel, the drop, the basket grid, map mode |
| 8 | **Stage 03** | `src/groups/*`, shell | F4a–F4c; tree, echelon, four refusals, ring pack on drop |
| 9 | **Stage 04** | `src/book.rs`, shell | F5a, F5b; steps, sub-steps, audiences, beats, idle bucket, nested basket, drop resolution |
| 10 | **Stage 05** | `src/roster.rs`, shell | F6a, F6b; people, seats, scopes, Not Playing, the two drop targets |
| 11 | **Stage 06** | `src/commit.rs`, shell | F7; the rail, the verbs, the failure block, resume |
| 12 | **Arrival** | shell | F8; three states, the gate strip, the server's blockers |

Dependencies: 1–3 are independent of the UI. 5 needs 1. 6 needs 1. 7 needs 1,
2, 5. 8 needs 2. 9 needs 4. 10 needs 4 (roles). 11 needs 3 and 4. 12 needs 3.

**This flow replaces the existing sessions modal and wizard.** `shell_gpui.rs`
already has `sessions_open`, `wizard_step`, `SessionDraft`, `compose_focus` and
a 25-slot focus-handle array behind them. The landing's left column is that
modal's list, kept; the wizard's five window fields (`WizardField`: day, real
start, real end, exercise start, exercise end) are replaced by stage 01's time
base (§4.1), and `terms.rs`'s five-field `Terms` grows to the ten fields plus
the pace. Delete the old wizard path in the same slice that lands stage 01 —
do not keep two ways to create a game.

`book.rs` currently has **no sub-step support anywhere** — its `plan` is
step-level. Extending it is slice 9's first job, and the model change is the one
that touches the most existing code.

---

## 16. Verification

Read `docs/agents/verification.md` before you verify anything. In its order:

1. **A unit test, first, always.** `cargo test --lib` is six seconds. Slices 1–3
   are pure and should never need a window.
2. **A capture, second.** `scripts/gpui-sweep.sh <tag> "name:seed|assertions"`.
   The harness prints `input during capture` and `TAINTED`; read that line
   before believing the picture.
3. **One probe per build.** Do not rebuild to answer a second question.
4. **Numbers, not pictures.** `scripts/shot-assert.sh` asks a capture a question
   and returns a number. A full-frame `read` is the most expensive call in the
   loop.

**Seeds.** Each frame needs a deterministic state, and the hosted API cannot be
made to hold "9 placed, 3 unplaced, 4 groups, 5 in them". Add one seed family,
`TFG_FLOW=<state>`, that stages the **draft** from a fixture without a write:

```
TFG_FLOW=f1a | f1b | f2a | f2b | f3a | f3b | f3c | f3d | f3e
         f4a | f4b | f4c | f5a | f5b | f6a | f6b | f7 | f8
TFG_FLOW_FAIL=seats          # stages F7's 502 at write 02, seats
TFG_BASKET=1                 # opens the basket; the way to photograph F4a's
                             # nested tree, which has no frame of its own
TFG_MAP=1                    # opens map mode for the same reason
```

A fixture capture is a picture of the fixture. It is the right evidence for
layout and for a gesture's intermediate state, and it is **not** evidence that a
write works. Each ladder position needs one live run against the hosted API, and
the report should say which positions were driven.

**A UI slice is not done until it has been driven.** The compiler does not see a
window, and this flow's defects are drag defects.

---

## 17. Out of scope

- Reopening a game that is not in `planning`. Proceed opens the console instead;
  the terms, units and tree are fixed after planning.
- Handing a scope over from one person to another (one refusal, no dialog).
- Moving a hull between command scopes after the game has begun.
- Editing `overlay_text`.
- Persisting the draft, or a draft history.
- Deleting a game, duplicating one, or replaying one (the landing's rows do not
  offer them).
- Per-step readiness, running a step, rewinding — the execution console's.
- Template authoring in the CMS. This flow creates Saved Scenarios; it does not
  edit the CMS's role vocabulary.

---

## 18. What is settled, and the one question left

Six facts this document needed from the live API. Five were driven on
**25 Oct 2026**; one is still open.

| # | Question | Answer | Where it lands |
| --- | --- | --- | --- |
| 1 | `max_echelon_rank` — hierarchy id or rank number? | **A rank number.** Send `40`. | §12.7, §3.3, §6.1, §11 |
| 2 | `POST /games` with client-computed `actual_end` / `assumed_end`? | **Accepted.** Send both ends. | §4.1, §12.8 |
| 3 | `PATCH /games/{id}` with `time_factor` — does it move `assumed_end`? | **No. Send both** `time_factor` and the new `assumed_end`. | §4.1 |
| 4 | `GET /games/{id}/readiness` — does the read answer? | **Yes**, despite the spec listing only `PUT`/`DELETE`. 08 may depend on it. | §9.2, §10 |
| 5 | `GET /units/asset-groups` — is `Aset Lainnya` returned at zero, and does `/unit-categories?id_asset_group=` take its id? | **Open.** | §5.1 |
| 6 | `POST /scenario-missions` reachable, and may `code` be omitted? | **Yes to both.** | §9.2, M1 |

**The one open question is the cascade's first rung.** Check it before slice 7:
that `GET /units/asset-groups` returns all four groups (including an empty
`Aset Lainnya`) and that `GET /unit-categories?id_asset_group=` accepts the id
the group carries. If either fails, render the groups the endpoint does return
and drive the second rung by whatever key it accepts — the layout does not
change, only the filter's name.
