# Product

<!-- impeccable:product-schema 1 -->

## Platform

desktop

<!-- Note: `desktop` is outside the web/ios/android/adaptive enum by explicit
     user decision (2026-09-23): the app is a native Rust/egui desktop window,
     and none of the four schema values is truthful for it. -->

## Users

Primary: the people running and playing in a live wargaming exercise, each at
their own machine in an exercise room or the field —

- **Organizer** — sets up a Session (time windows, roster, placements, seats,
  groups), commands all units, runs Setup → Live → Closed.
- **Commander** — issues Commands over the units in their jurisdiction
  (higher ranks override lower; the organizer commands everything).
- **Helm** — drives one unit (the seat that makes a unit playable).
- **Observer** — watches; view-only.

Confirmed: no other audience has been established.

## Product Purpose

A presentation client for a tactical floor game: it shows where units are on a
real-world tile map while an exercise session runs, and lets seats act within
their authority (orders, commands, setup). Success is an exercise room able to
run a whole session — setup through closure — on shared live map positions,
with a replayable journal of what happened.

## Positioning

One fix registry ingests position reports regardless of provenance — wire
(from the Minos backend) or sim (synthetic, emitted from orders) — so live
data, simulated units, and replay all render identically in presentation. The
client never simulates where anything goes; it only presents accepted fixes.
A neighboring product could not copy this by bolting a simulator onto a map:
the provenance-agnostic registry, and the sim acting as just another fix
source, is the mechanism.

## Operating Context

- Exercise room / field use: mixed desk monitors and field laptops, varied
  screen sizes, glare, less-controlled lighting.
- Backend-optional: reads live from the Minos backend when connected (user
  directory, game participants, realtime feed); local-first otherwise, with
  mock/replay backends for dev and air-gapped runs.
- Air-gapped delivery: SQLite amalgamation compiled in (no system library),
  vendored socket SDK, assets shipped in-repo.
- Real-time sessions over a websocket feed; UI thread never touches async
  (dedicated runtime thread in the actor).
- Bilingual domain vocabulary in daily use: English interface terms plus
  established Indonesian organizational terms (Unsur, Satuan Tugas, Gugus,
  Operasi Gabungan, TNI AL).

## Capabilities and Constraints

Binding product facts (confirmed as durable, not revisable implementation
detail):

- **Air-gapped / local-first delivery** — bundled SQLite, vendored deps, no
  system libraries; the local store holds state when no backend is reachable.
- **Backend-optional operation** — live, replay, and mock backends are
  interchangeable sources; local drafts survive disconnection.
- **egui as the UI shell** (ADR-0002) — all client UI in egui/eframe.
- **MapLibre as a pure map engine with game objects as overlays**
  (ADR-0001) — tiles and camera only; markers/trails/zones are client-side
  overlay primitives, never map style layers.
- **Game time = real time × fixed ratio** (ADR-0004) — `game_now = game_start
  + elapsed_real × ratio`; pause freezes game time, never the wall clock, and
  session datetimes are never edited.
- **The CONTEXT.md vocabulary is canonical** — the glossary's terms and
  avoided synonyms bind all product writing and code vocabulary.

Confirmed functionality: session lifecycle (Setup → Live → Closed), roster and
seats with invites, group hierarchy with per-level commanders, jurisdictions
and authority, orders fanning out from commands, unit taxonomy
(Category/Class/Type), zones and flags, islands (floating panels) over a
fullscreen map, wizard-based setup, and an append-only replayable Log.

Open (recorded, not decided): Minos-side Closure-phase analysis features
(judging, playback scoring) are backend concepts — whether/where this client
exposes them is undecided. Invite redemption over the network is a later
slice (currently local/mock).

## Brand Commitments

- Bilingual interface: English UI chrome and copy, with Indonesian
  organizational terms kept as domain vocabulary; both languages are supported
  (user decision, 2026-09-23).
- Canonical vocabulary lives in `CONTEXT.md`; new writing must use its terms
  and honor its `_Avoid_` lists.
- Product naming in evidence: package/crate `tfg` (Tactical Floor Game);
  the connected backend is Project Minos.

## Evidence on Hand

- `CONTEXT.md` — full domain glossary (language, roles, phases, groups).
- `docs/adr/0001–0007` — rendering seam, egui shell, sim-as-fix-source,
  game time, unit taxonomy, TNI AL fleet import, unified group model.
- `minos-docs/` — backend concept (`concept-en.txt`), `game-modes.md`,
  `backlog.md`, `realtime-protocol.md`, `beacon-protocol.md`,
  `unit-classification-mapping.csv`, OpenAPI specs, `client-questions.md`.
- `docs/minos-api.yaml`, `docs/Minos Tactical Floor Game API.json`.
- `assets/fleet.json`, `assets/catalog.json`, `assets/ne_50m_land.json`,
  `assets/tiles-cache.seed.sqlite`.
- `scenarios/{empty,surge,ghost,dark}.json`, `tests/fixtures/tracks.json`.
- `examples/` — `egui_window`, `mock_backend`, `map_spike`,
  `calibrate_projection`, `seed_cache`, `offline_check`.

Absences future work must not fabricate: no testimonials, customers,
benchmarks, press, pricing, or licensing evidence exists in this repo.

## Product Principles

1. **Present, never predict.** The client renders accepted fixes; simulation
   is someone else's job (the sim is just another fix source).
2. **Authority is explicit.** Every action flows through seat, jurisdiction,
   and phase — if the model doesn't permit it, the UI doesn't offer it.
3. **Offline is a first-class state.** Air-gapped and disconnected operation
   is the norm, not a degraded mode; local state is real state.
4. **The record is the product.** The append-only Log and replayable journal
   make any session auditable after the fact; never mutate what happened.
5. **One vocabulary.** `CONTEXT.md` terms are the only terms — in code, UI
   copy, docs, and issues alike.

## Accessibility & Inclusion

- Mixed usage scene confirmed: desk monitors and field laptops, varied screen
  sizes, glare and less-controlled lighting — legibility under glare and
  across small/large screens is a real requirement.
- No product-specific accessibility standard (WCAG level, screen-reader
  support, etc.) has been established; recorded as undecided rather than
  assumed.
