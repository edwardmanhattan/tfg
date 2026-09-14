# Game time maps real time through a fixed ratio

Every session carries four real-world datetimes (`real_start/end`,
`game_start/end`); the game clock derives from them
`ratio = game_span / real_span`, fixed for the session, and maps
`game_now = game_start + elapsed_real × ratio`. Until a real `Game` model
supplies bounds, the sandbox runs at 1:1 with `real_start` stamped at the
first sim tick (grill #17).

## Considered Options

A mutable mid-session compression multiplier (rejected: previously logged
game timestamps would no longer match any single mapping — the log timeline
must stay coherent). Freezing `game_now` at `game_end` when the real clock
overruns (rejected: ships stop mid-ocean and the log desyncs from motion);
the ends are advisory and sessions end explicitly, never by the clock.
Editing the stored session datetimes on pause (rejected: corrupts the
session record); pause is a full hold enforced tick-wise — paused sim rounds
advance nothing while the wall clock runs on — equivalent to a pause
accumulator. Storing `game_ts` on each `Fix` as sketched in the game model
(rejected: it freezes the mapping into every fix and duplicates data that
cannot be un-derived).

## Consequences

Sim motion integrates over game elapsed (`distance = speed × game_dt`), so
the ratio actually moves ships. `game_ts` is derived — `game_now` is a pure
function of the start stamp plus game elapsed; wire fixes get mapped game
timestamps for free, keeping the future log (#20) coherent across
provenance. ETA is quoted in game time. The UI forwards pause (Space) but
never computes time itself; the sim reports both readings per round
(`real_ts`, derived `game_ts`). A 60× fixture proves the mapping in tests;
1:1 default leaves the approved #16 prototype behavior unchanged.
