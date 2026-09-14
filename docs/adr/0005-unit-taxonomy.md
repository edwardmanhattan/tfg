# Unit taxonomy: category = schema, class = abilities, type = flavor

Three levels; the sketched fourth (dev-type) is collapsed as a fuzzy
duplicate (grill #18, ADR-0005):

- **Category** (`Ship`/`Plane`/`Tank`/`Port`) determines WHICH stat keys
  exist — the schema. Ships have `speed_kn` (± `capacity_t`); ports would
  have `berths`; planes/tanks their own.
- **Class** (Tanker, Destroyer, Container…) holds the stat VALUES — the
  abilities. All units of a class behave identically in the sim; class
  speed caps ordered speed.
- **Type** (`Type 052D`, `Arleigh Burke`) is the player-facing designation.
  Flavor only: no mechanics, many types per class.

## Considered Options

Keying abilities off category (rejected: every ship would sail identically
— a tanker at destroyer speed). A separate dev-type level (rejected: with
class = abilities, dev-type duplicated it under a vaguer name; the user
collapsed it). Per-category typed stat structs (rejected by decision:
a generic `HashMap<String, f64>` keeps the model data-driven; instead a
load-time validation enforces each class carries its category's required
keys and fails loud). Taxonomy on wire/traffic ships (deferred: traffic
stays raw fixes until identification mechanics exist).

## Consequences

`Unit` (catalog entry: taxonomy + stat row) and `GameUnit` (instance:
position/state/order) are separate — a fleet is N GameUnits referencing
one Unit. The catalog lives in `assets/catalog.json` (data-driven like
scenarios), loaded at startup; take-control presents a class selector and
the chosen class's stats drive the unit from then on. Ship-only is
implemented: Plane/Tank/Port are reserved in the enum without stat rows or
behavior.
