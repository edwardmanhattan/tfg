# Unified group model: one struct plus a ranked level enum

Satgas and Gugus were two full structs differing only in member type
(units vs satgas ids), with near-identical add/remove/validation paths.
The opening proposal was a shared trait; the map decided otherwise
(tickets #71–#75, spec #76):

- **One `Group` struct** (`id`, `name`, `kind`, `commander`, `units`,
  `children`) plus **`GroupKind::{Unsur, SatuanTugas, Gugus,
  OperasiGabungan}`** with explicit gapped ranks (10/20/30/40).
- Nesting is generic: any group may hold units, child groups, or both;
  every child sits at a strictly lower rank (cycles impossible by
  construction). Unit placement is a nullable leaf assignment; forests
  (multiple roots) allowed.
- Authority is rank-derived: max covering rank wins, organizer supreme.

## Considered Options

A shared `Group` trait with per-level structs (rejected: the level
vocabulary is closed in-repo master data, not an open plugin surface —
the trait buys openness nobody needs, costs parallel impls at ~40 UI
call sites, and loses exhaustive `match`, which lists every site a new
level affects). A fixed two-level Satgas/Gugus pair (rejected: Minos
carries four levels plus middle insertion — a future Koarmada between
Gugus and Operasi Gabungan — and ordinal ranks would renumber on every
insertion). Per-kind submodules (rejected: resurrects the same
duplication at file level; `groups/` splits per responsibility —
`model` / `hierarchy` / `authority` / `scopes` — mirroring `backend.rs`).

## Consequences

New levels are a data row (variant + rank), never a module or a
renumber: insertion takes a midpoint. `command::Authority` carries group
ranks (`UNSUR 10`, `SATGAS 20`, `GUGUS 30`, `OPERASI_GABUNGAN 40`,
unit-direct 0, organizer max); journal `authority:<rank>` readings
change accordingly. Scope ids generalize to `kind:id`. The glossary
canonicalizes Satuan Tugas (`Satgas` stays a local alias only).
Migration was flag-day: `add_satgas`/`add_gugus`, the `satgas_*` /
`gugus_*` accessors, and the fixed three-level authority mapping are
gone with no compat wrappers.
