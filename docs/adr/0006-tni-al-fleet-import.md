# Fleet import: TNI AL sheet as catalog + placements seed

The placeholder catalog (Tanker/Destroyer/Container) is replaced by the
real fleet: `Aset Kapal TNI AL.xlsx` (125 active hulls, Sept 2026),
via `scripts/fleet_import.py` into `assets/catalog.json` (41 classes)
+ `assets/fleet.json` (125 GameUnit seeds). Task #28.

## Considered Options

Hand-authoring classes for the real fleet (rejected: 41 classes × stats
is transcription busywork, and the sheet already carries per-hull
Max/Cruise/Range — siblings agree, so class-level mode aggregation is
faithful). Promoting sheet categories (Frigat, KCR, Amfibi…) to
`Category` (rejected for now: all hulls are `Category::Ship`; the sheet
categories are class families, and the sim has no per-category behavior
yet — confirmed with the user). Per-hull stats instead of per-class
(rejected: violates ADR-0005, class = abilities; the sheet confirms
siblings share numbers).

## Consequences

- `speed_kn` stays the only required (sim-read) stat: the order cap.
  `cruise_kn`/`range_nm` ride along as validated capabilities for the
  next slice (default transit speed, patrol radius); `endurance_days`,
  `crew`, `disp_full_t`, `loa_m` are optional. Weapons/sensors/propulsion
  prose stays display-only in fleet seeds (the stat map is numeric).
- Sheet `Kelas` → Class id (slug), `Tipe/Peran` values → flavor `types`;
  hulls (name + number) → fleet seeds, never Types.
- Home-base coords are administrative (dermaga), seeded as placements,
  never mistaken for live fixes (the sheet says so explicitly).
- Known quirks preserved loudly: Lumba-Lumba 858 twice (second id gets
  `-2`), Teluk Berau 527 (Bintuni) vs 534 (Frosch — hull distinguishes),
  tail rows skip column AA (the importer parses by cell reference).
- Wiring fleet seeds into session setup (placements, roster) is a later
  slice; the loader only parses + validates, with a test that every
  seed resolves to a catalog class.
