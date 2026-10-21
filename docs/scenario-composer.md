# The scenario composer

A ticket for the composer: the library, the authoring surface, and the two
datetimes. Written before the code, because the shapes here are what three
pieces of work (the library, the calendar, the console surface) have to agree
on, and a disagreement found in the console is a disagreement found late.

## The three things

| Thing | Where it lives | What it is |
| --- | --- | --- |
| **Saved scenario** | this machine, `scenarios.json` under the data dir | A whole scenario the operator authored, kept across games. |
| **The composer** | the console, in memory | The scenario being authored right now: name, description, window, steps. |
| **The book's scenario** | Minos, one per game | What the server holds: title, description, and its steps. |

The three are not one thing and must not be drawn as one. The saved scenario is
a PLAN the operator keeps; the book's scenario is a DEPLOYMENT of it in one
game. Minos has no catalog of scenarios — a game has exactly ONE book, and the
book is its ordered scenarios — so a saved scenario is a client-side fact, and
the client is the only place it can live.

## Why a client-side library at all

Because "compose a scenario, then choose it in another game session's planning"
is a request the server cannot answer: `POST /games/{id}/scenarios` writes into
one game's book, and there is no scenario template anywhere in the Minos
contract. The catalog is therefore OURS: a JSON file beside the local store,
holding the plan whole — name, description, window, steps, and each step's
place and its related hulls.

The consequence is a rule the composer must not break: **the library is the
source of truth for a plan, and the book is a deployment of it.** A saved
scenario pushed into a game becomes that game's scenario; the game's copy is
what the server holds, and the library keeps its own.

## The data (crate `tfg::library`)

```rust
/// Where a step points, when the author dropped a pin.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Place { pub lat: f64, pub lon: f64 }

/// One step of a saved scenario.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SavedStep {
    pub content: String,
    /// `HHMM` on the assumed clock. Both ends or neither.
    pub start_hour: Option<String>,
    pub end_hour: Option<String>,
    /// The place the step points at, when the author dropped a pin.
    pub location: Option<Place>,
    /// The hulls (and task-organisation nodes) the step obliges, in the
    /// book's own shape: `(kind, id)`, kind being "hull" or "node".
    pub related: Vec<(String, i64)>,
}

/// One saved scenario: the plan, whole.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SavedScenario {
    /// The library's own key. Never a server id: a saved scenario outlives
    /// every game it was ever pushed into.
    pub id: String,
    pub title: String,
    pub description: String,
    /// The plan's window, as a LOCAL datetime: `YYYY-MM-DDTHH:MM`, no zone.
    pub start: Option<String>,
    pub end: Option<String>,
    pub steps: Vec<SavedStep>,
}

/// The whole library as it is held on disk.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Library { pub scenarios: Vec<SavedScenario> }
```

### The API (exact signatures — the console is written against these)

```rust
impl Library {
    /// Read the library. A missing file is an EMPTY library, never an error:
    /// a machine that has never saved a scenario has no library, and that is
    /// not a failure. A malformed file is also empty — a corrupt file must
    /// not take the console down with it — and the caller is told by
    /// `last_read_failed`.
    pub fn load(path: &Path) -> Library;

    /// Write the library. Atomic: a temp file beside the target, renamed
    /// over it, so a crash mid-write leaves the old library intact.
    pub fn save(&self, path: &Path) -> Result<(), String>;

    /// One saved scenario by id.
    pub fn get(&self, id: &str) -> Option<&SavedScenario>;

    /// Put a scenario into the library, replacing the one with the same id.
    /// A scenario with a blank id is GIVEN one. Answers the id.
    pub fn upsert(&mut self, scenario: SavedScenario) -> String;

    /// Take a scenario out of the library. Answers whether it was there.
    pub fn remove(&mut self, id: &str) -> bool;

    /// The library's scenarios in a stable order: by title, case-blind, then
    /// by id, so the list never reshuffles between two frames.
    pub fn sorted(&self) -> Vec<&SavedScenario>;

    /// Whether the last `load` found a file it could not read.
    pub fn last_read_failed(&self) -> bool;
}

/// A library key that cannot collide with another: the wall clock in
/// milliseconds, then a counter for two saves in the same millisecond.
pub fn new_id() -> String;

/// The place a step points at, as the step's words carry it: `at {lat:.4},
/// {lon:.4}`. THE SAME DIGITS as the Inspector reads a fix back with — an
/// exercise authored against one position and reviewed against another is a
/// scenario nobody can trust.
pub fn place_text(place: &Place) -> String;
```

### Pushing a saved scenario into a game

The composer stages the plan into the game's book draft, and the advance
writes it through — the same rule every other draft edit obeys (see CONTEXT.md,
**Book draft**): a create before its steps, and the steps in authoring order.

```rust
/// Stage a saved scenario into a book draft: the scenario, then its steps,
/// then each step's related list. Answers the staged scenario's local id.
pub fn stage_into(draft: &mut crate::book::BookDraft, saved: &SavedScenario) -> i64;
```

The staged scenario's TITLE and DESCRIPTION go to the server (Minos holds both).
The window does not: **Minos has no datetime on a scenario.** A scenario's
window in the library is the operator's PLAN — when this scenario is to be run
— and the server's timing for it is the STEP windows on the assumed clock. The
composer offers the plan's window as the DEFAULT for the first and last step's
window, and the operator sees it offered rather than it happening behind them.

## The calendar (crate `tfg::calendar`)

Toolkit-free, so the month grid is testable with no display.

```rust
/// A month, as a year and a month number (1..=12).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Month { pub year: i32, pub month: u32 }

impl Month {
    pub fn new(year: i32, month: u32) -> Month;      // the month is normalised into range
    pub fn now() -> Month;                            // the local month
    pub fn grid(&self) -> Grid;                       // the weeks, Monday first
    pub fn next(&self) -> Month;
    pub fn prev(&self) -> Month;
    pub fn label(&self) -> String;                    // "October 2026"
}

/// A month's days, in weeks. Six weeks always, so the calendar's height does
/// not change from month to month — a mini-calendar that grows and shrinks
/// under the pointer is a calendar nobody trusts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Grid { pub weeks: Vec<Vec<Option<u32>>> }
```

The datetimes are local and zoned-free, because a scenario's window is a plan
the operator writes down, not an instant the world agrees on. They are written
`YYYY-MM-DDTHH:MM`.

```rust
/// Parse a local datetime. `None` for anything else, including a zone.
pub fn parse(s: &str) -> Option<NaiveDateTime>;

/// Write a local datetime: `YYYY-MM-DDTHH:MM`.
pub fn format(dt: &NaiveDateTime) -> String;

/// The local now.
pub fn now() -> NaiveDateTime;

/// Whether an end is after its start. A window with one end is legal
/// (a scenario may have a start and no end); a window whose end is not after
/// its start is not.
pub fn window_ok(start: Option<&str>, end: Option<&str>) -> bool;

/// The datetime at a given day of a month, keeping the time of day.
pub fn at_day(dt: &NaiveDateTime, day: u32) -> Option<NaiveDateTime>;
```

## The console surface

The composer's island is the Book panel (the door already says "Scenario
composer"). It carries three sections, in the order the operator works in,
and the third is empty until one of the first two has given it a scenario.

1. **New scenario** — the form that creates one: the name, the description,
   and the window as a DATE and a TIME for each end. The button that creates
   it is at the end of the section: creating is a separate act from typing,
   because a scenario is what the steps belong to.
2. **Saved scenarios** — the library: each saved scenario with its window and
   its step count, and two verbs: *use* (open it in the third section) and
   *delete* (take it out of the library).
3. **Steps** — the steps of the scenario being worked on: its name and
   window at the top, the step list, and the row that adds a step. It is
   only there when a scenario is open, because a step belongs to a scenario
   and a step list with nothing above it is a list of orphans.

### The datetimes, and why a mini-calendar and a time grid

Typing `2026-10-22 08:00` is a test of memory, and a typo in a date is a
silent wrong answer — the console would take it and the operator would not see
it for a week. The fields take the typing, and two grids take the choosing:
a **month grid** for the day, and an **hour-and-minute grid** for the time.

Rules the fields obey:

- The date and the time are SEPARATE fields. A day is a shape and is
  chosen; an hour is a number and is typed. One field for both would ask the
  operator to remember a choice they had already made.
- The grids write into the same fields the keyboard writes into, so the two
  can never disagree.
- The month grid is always six weeks, so the control's height never changes
  from month to month.
- The hour grid is twenty-four hours six to a row; the minutes are in fives,
  because the quarter-hour is what an exercise is timed on — and the field
  still takes any minute the operator types.
- An end that is not after its start is refused in the FIELD, in the
  composer's own sentence — not after the round trip.
- A scenario may have a start and no end, or neither.

### The step maker

A step is three things, and the composer shows all three:

- **The words** — a text field.
- **The place** — a pin, dragged onto the water or armed and clicked. The pin
  writes `at {lat:.4}, {lon:.4}` INTO the words (the same digits the
  Inspector reads back), and the composer ALSO keeps the place structurally, so
  the library can show it and the composer can point at it.
- **The hulls** — the hulls of the exercise the step obliges, picked from the
  force. A step that names no hull obliges nobody, which is legal and says so.

### The verbs

| Verb | What it does |
| --- | --- |
| **Create scenario** | Makes the scenario the form describes. Nothing is written: the scenario is the operator's own until it is saved or added to a game. |
| **Save to library** | Writes the open scenario into the library, under its id (a new one if it has none). Answers "saved". |
| **Use** | Opens a saved scenario in the steps section. Nothing is written: the operator edits it and saves it, or adds it to the game. |
| **Add to this game** | Stages the open scenario into the game's book: a scenario, its steps, its relations. Written through at the advance. |
| **Delete** | Takes a saved scenario out of the library. The library only — a scenario already pushed into a game is the server's, and this verb does not touch it. |

## The rules that hold everywhere

- **The library never writes to the network.** A saved scenario is local, and
  the only thing that reaches Minos is a stage advance.
- **A step's window is both ends or neither.** One rule, from
  `crate::window::window_problem`.
- **The staged draft is what the operator sees.** A scenario staged for a
  game stands in the book marked, until the advance writes it.
- **Nothing is deleted silently.** A scenario leaves the library when the
  operator deletes it, and the game keeps its own.
