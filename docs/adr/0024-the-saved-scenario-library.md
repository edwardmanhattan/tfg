# ADR-0024: The scenario library is this machine's own

A scenario the operator composes — a name, a description, a window, its
steps, the place a step points at, the hulls it obliges — was to be chosen
again in another game session's planning. The composer had a step maker and
a pin and a related-hull list, and no way to keep what it had written.

## Status

Accepted. Carries forward ADR-0017 (the force draft is local and written
through at an advance) and ADR-0020 (the console is one GPUI shell).

## Context

**MinOS has no catalog of scenarios.** A game has exactly ONE book, and the
book IS its ordered scenarios: `POST /games/{id}/scenarios` appends to that
one game's list, and there is no scenario template, no cross-game scenario
resource, and no endpoint that lists scenarios outside a game. The client
says so in the code that talks to it (`backend/master.rs`, "the scenario
book").

So "save this scenario and choose it in another game" has nowhere to go on
the wire. Three answers were possible:

1. **Push a copy into every game.** The console would POST the scenario
   into a game the operator is not planning, at the moment they saved it.
   That is a write into somebody else's exercise, from a place that has no
   business writing there, and it fills a game's book with plans the
   operator did not choose to put in that game.
2. **Keep it in the client only, and never write it.** The scenario is
   authored in the game it belongs to, and a scenario for another game is
   re-typed. This is the console's answer today, and it is the one the
   operator asked to have replaced.
3. **Keep the catalog in the client, and DEPLOY it into a game when the
   operator says so.** The library is a file on this machine; adding a
   scenario to a game is a separate, deliberate act, and it goes through
   the book draft like every other authoring edit.

The third is what the console does now.

**MinOS has no datetime on a scenario either.** The scenario record is a
title, a description, and its steps; a step is free text plus an optional
`HHMM` window on the assumed clock. There is no date and no time on a
scenario, so "when does this scenario run" is not a fact the server can
hold. It is a fact about the PLAN, and the plan is the operator's.

## Decision

**The library is a client-side file, and it is a plan; the book's scenario
is a deployment of it.**

- **The library is one file under the data dir** (`scenarios.json`, beside
  the local store), read once at boot and written atomically. A missing file
  is an EMPTY library and never an error: a machine that has never saved a
  scenario has no library, and that is not a failure. A malformed file is
  also empty, and says so — a corrupt file must not take the console down
  with it.
- **The library never talks to the network.** A saved scenario is the
  operator's own, and Minos is not told about it until it is added to a
  game. There is no POST behind "save".
- **Adding a scenario to a game is a DRAFT edit**, like every other
  authoring act (ADR-0017): it stages into the Book draft and the stage
  advance writes it through — the create first, because a step belongs to a
  scenario the server has to hold first, and then the steps in authoring
  order, and then each step's related list.
- **A scenario's window is a plan, and the server's timing is the step
  windows.** The plan's datetime is local, typed and zoned-free
  (`YYYY-MM-DDTHH:MM`), and it is said out loud where it lands: the first
  step takes the start and the last step takes the end, on the assumed
  clock. A step with a window of its own keeps it — the author's own timing
  is more precise than the envelope.
- **A place is text AND a number.** The pin writes `at {lat:.4}, {lon:.4}`
  into the step's words — the same four decimals the Inspector reads a fix
  back with, because an exercise authored against one position and reviewed
  against another is a scenario nobody can trust — and the library keeps the
  place beside it as a number, so a step that points somewhere can be found
  again without parsing prose.
- **A related hull is a step's obligation, and it is named by the exercise
  it belongs to.** A saved scenario carries the hulls its steps oblige; a
  hull that is not in the game the scenario is deployed into is not in that
  game, and the deployment writes the relations the game can hold.

## Consequences

- The scenario a game holds and the scenario the operator kept are two
  things, and the console draws them as two things: the library is a
  section of its own, and the deployment is a staged row in the book.
- A plan survives the exercise it was written for, which is the point: a
  Game Master who wrote a good crossing in October can use it in March.
- The library is this machine's file and not the server's: it does not
  travel to another machine, and two operators do not share a catalog. That
  is the cost of a server with no catalog in it, and it is recorded here
  rather than discovered later.
- A scenario deleted from the library is gone from the library only. A
  scenario already deployed into a game is the server's, and deleting the
  plan does not delete the deployment.
- The date is CHOSEN and the time is TYPED, because a minute is a
  keystroke and a month is a shape. The mini-calendar writes into the same
  field the keyboard writes into, so the two can never disagree.

## Verified

Against the deployed dev API, 2026-10-21, on a planning scenario exercise:

- A scenario composed in the composer, saved, and picked again from the
  library comes back with its name, its window, its steps, its place and its
  related hulls.
- Adding it to the game stages a create and its step adds; the advance
  writes them in that order, and the game holds the scenario with its steps.
- The library file is written atomically and re-read at boot; a deleted
  file leaves an empty library and a live console.
