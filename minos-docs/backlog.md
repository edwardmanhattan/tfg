# Backlog

Known, deferred. Not blocking, not forgotten. Each entry says what was observed, not only what
was concluded — an observation survives a wrong hypothesis.

---

## B1 — Intermittent test failure while the dev server is running

**Observed.** One `go test ./... -count=1` run reported 1 failing test while the dev server was
running against the same database. Three consecutive runs immediately afterwards, with the server
stopped, were clean (245 passing, 0 failing). The failing test's name was not captured, which is
the main thing missing here.

**Likely cause, unconfirmed.** The DB-backed tests mutate shared state against one database —
`pkg/casbin`'s `cleanTable` rewrites `casbin_rule` wholesale, `pkg/migration` seeds and purges real
accounts, `internal/repositories` writes real rows. A live server holds an in-memory enforcer that
then disagrees with the table underneath it. Running the suite and a server concurrently is
therefore not a supported combination, but nothing says so.

**Sharpened 2026-09-15, and the workaround now has a reason.** `cleanTable` deletes every policy for
the DURATION of each test and restores them afterwards (it no longer restores a hardcoded list — see
B8). So for the length of any test run the table is legitimately empty, and a server that reloads
policies from it inside that window drops every permission it holds. That is a concrete mechanism
for "one test failed while the server was running", and it survives the B8 fix: the empty window is
the point of the isolation, so "stop the server first" stays the rule rather than a superstition.

**Next step.** Capture the test name by keeping the output next time it happens. If it turns out to
be `pkg/casbin`, the fix is probably for those tests to refuse to run when another connection holds
the table — or for the suite to be documented as "stop the server first".

**Workaround for now:** stop the dev server before running `go test ./...`.

---

### Captured 2026-09-15 — the test name, and a likelier cause than the dev server

**Test:** `TestSyncRolesIsIdempotent` in `pkg/casbin`, failing with

```
insert or update on table "user_roles" violates foreign key constraint "user_roles_id_user_fkey"
```

**What the error says.** `newSyncFixture` INSERTs its own user row and then writes a grant against
it, so this failure means the user row was already gone when the grant was written. The fixture's
own cleanup is scoped — `DELETE FROM users WHERE id = $1 AND created_by = 1` — so it did not remove
itself. Something else did.

**So the dev server was probably never the mechanism.** `go test ./...` runs packages
CONCURRENTLY, and `pkg/migration` seeds and purges real accounts against the same database. A
package-wide purge landing between the fixture's INSERT and its grant deletes the user out from
under the test. Two parallel runs immediately afterwards were clean, which is precisely the
intermittency described above — and it reproduces with no server running at all, which the original
entry never checked.

**Almost-certain fix:** run the DB-backed packages serially, or give each package its own database
so they cannot reach into one another. `go test -p 1 ./...` passed 10/10 immediately after the
failure. Documenting "stop the server" was treating a symptom.

**To confirm:** run `pkg/casbin` in a loop in isolation (expect clean) and the full parallel suite
in a loop until it reproduces (expect the same FK error). The package that purges accounts is the
one to isolate.

**Workaround for now:** `go test -p 1 ./...`.

---

### Captured 2026-09-17 — a SECOND failing test, and a rate

**Test:** `TestSeedCreatesTheAccountWithAForcedPasswordChange` in `pkg/migration`, failing with

```
the account was created without the Administrator role, so it cannot do the one job it exists for
no rows in result set
```

**Rate: 1 failure in 6 runs** of `go test ./... -count=1`. It passes in isolation, passes when the
package is run alone, and passed 3/3 with `-p 1`. So it is the parallel-package interference above,
not a defect in the seed.

**A mechanism the earlier notes did not name.** They attribute the symptom to a *live server*
reloading policies during `cleanTable`'s empty window. This failure had no server running: the
conflict is between two TEST PACKAGES. `pkg/casbin`'s `cleanTable` commits `DELETE FROM casbin_rule`
and restores it in a `t.Cleanup`, so for the duration of each of its tests the table is empty and
visible to every other connection — and `pkg/migration`'s superuser seed reads the Administrator
grant back from that table. The seed inserts, finds no grant, and reports the account as
unusable.

**So B8's fix did not close this.** Snapshot-and-restore made the *end* state correct; the empty
window in the middle is deliberate isolation and remains. The residual B8 records — "the table is
still empty for the DURATION of each test" — is therefore not only a hazard for a concurrent
server, which is how B8 framed it.

**Next step.** Either serialise the DB-backed packages, or give each package its own database. The
latter is the only fix that preserves parallel runs. Worth noting that a killed process between the
delete and the cleanup leaves the policies gone for good — `migrator up` will not restore them,
because the migration that installed them is already recorded as applied (B8).

**Narrowed 2026-09-21.** The suite runs on **CI only** and is never pointed at the dev database, so
the alarming reading — a test run wiping a shared environment's policies with no way to repair them —
is off the table. What remains is narrower and still worth fixing:

- **CI flakiness.** 1 failure in 6 runs makes a red build ambiguous, and a deadline is the worst time
  to be unsure whether a failure is real. This is the part that still matters.
- **A local hazard, not a deployed one.** `cleanTable`'s window only bites when a server is running
  against the same database as a concurrent test run, which is a development-laptop situation.

The fix is unchanged: serialise the DB-backed packages (`-p 1` is already the workaround) or give each
package its own database. The former costs parallel wall-clock; the latter costs a database per
package. CI that runs `-p 1` is the cheap answer and makes a red build mean something.

---

## B2 — Conflict payload shape is inconsistent

The unit conflict response returns `errors[field]` **plus** a machine-readable `data.id_unit`, so
the CMS can link to the conflicting hull. The unit-class refusal reports its hull count only inside
the prose of `errors._request`.

Both are contract-compliant, and they disagree with each other. One of them should change: a client
should not have to parse English to decide what to show.

**HALF CLOSED 2026-09-21 — the class half is gone rather than fixed.** The refusal that reported its
hull count only inside prose no longer exists: retiring a class now CASCADES to its hulls and
reports the same count as a number (`data.units`, alongside `classes`), which is what the second
shape asked for. The taxonomy retirement response is therefore the answer to this entry, and
`RetirementBlocked` documents the one remaining refusal — the `execution` guard — with BOTH a prose
sentence under `data.errors._request` and the same number as `data.units_in_execution`.

**Still open for `POST /units`**, which was the other half of the disagreement: its hull-number
conflict carries `data.id_unit` and its field errors together, and nothing has been changed there.

---

## B3 — `commissioning_precision: "unknown"` accepts years

The contract specifies the pairing rules for `range`, `exact`, `year` and `decade`, and is silent
on `unknown`. Only the written rules are enforced, so a year supplied alongside `unknown` is stored
and means nothing to any client. Rejecting it could refuse data the importer legitimately holds, so
it was not invented — but it should be asked.

---

## B4 — `turn_rate_is_derived` when nothing was derived

If `turn_rate_max_deg_s` is omitted **and** the hull's class has no default either, the rate is left
null and `turn_rate_is_derived` is `false`. Nothing was derived, so `false` is the honest value —
but the contract does not cover this combination explicitly. Worth confirming.

---

## ~~B5 — `is_judge_side` is not reachable from the CMS~~ — RESOLVED

**Resolved 2026-09-15.** The flag is now part of the shared lookup shape, so `/helpers` returns it
and both frontends can read it. Migration `000012_lookup_judge_side` adds the column to the seven
tables that lacked it; `HelperPublic` exposes it.

Normalised across every lookup rather than scoped to `game_roles`, because the ten tables share
one column list and one scan and that sharing is what stops the SELECT order drifting from the Scan
order. On the nine others the value is always `false`, and `false` is the truthful answer — a unit
status is not a role anyone holds. Same trade `000004` made when it added `description_en` and
`is_system` to every lookup.

**Still open, and separate:** nothing can *write* a lookup row. There are no write endpoints for
most of the ten tables, so an operator cannot add a role through the CMS at all — a new role can
only arrive by migration, and it would default to `is_judge_side = false`, meaning it is required
to declare readiness. That is a gap in the CMS's master-data surface, not in this flag, and it
predates slice 2.

**Partly closed 2026-09-17.** `POST`/`PATCH`/`DELETE` now exist for `/unit-categories` and
`/unit-types`, granted by migration `000024_taxonomy_authoring`, with audit columns added to both
tables. That is **two of the lookup tables**, and it is the pattern the rest can follow: a migration
for the audit columns and the grants, a write-only repository, service methods, routes, and tests.

The remaining unwritable lookups are `user_statuses`, `service_branches`, `movement_domains`,
`app_roles` and `deployment_statuses`. `unit_hierarchies` was already writable and is the
precedent the taxonomy followed.

**The DELETE verb is settled — see B10.** It became RETIRE with a cascade, and G2's *"lookup tables
keep exactly one delete verb: none"* was deliberately reversed there. So the paragraph above is the
pattern for the audit columns and the write routes, and the delete verb is not part of it any more.

**Two counts in this entry were wrong and are corrected 2026-09-21.** `models.HelperTables` holds
**ten** entries, not the eight named above — `unit_categories` and `unit_types` were added to it by
this entry's own work in `000024`, and `deployment_statuses` by `000021`. The number drifted as
tables were added and nobody re-counted. The two comments in `internal/services/helper_service.go`
that said "seven round trips" had drifted the same way and were corrected in the same pass; a wrong
count in a comment is worse than no count, because a reader trusts it.

---

## B6 — `unit_spec_versions` is vessel-shaped, AD/AU expansion is confirmed

C4 answered that Army and Air Force integration is planned, with class-table inheritance from day
one. `unit_spec_versions` is a single vessel-shaped table — `draft_m`, `speed_max_submerged_kn` —
so an aircraft's specification would carry columns that cannot apply to it.

Splitting later is mechanical-ish because versions are already keyed `(id_unit, version)` and `units`
is service-agnostic through `id_service_branch`. But it is cheapest before more code depends on the
current shape, and game execution is about to depend on it heavily.

---

## B7 — `static` mode is absent from the contract

`static` is deliberately not in the `GameMode` enum rather than present and unimplemented (H8 asks
whether deferring it is acceptable). If it is in the acceptance criteria, it needs scheduling
rather than discovering.

---

## B8 — The casbin test helper erased policies installed by later migrations (FIXED)

**Observed 2026-09-15.** After a `go test ./...` run, `POST /games` and every other game route
answered 403 for the administrator. `schema_migrations` reported `12` and clean, and every table was
present — the only thing wrong was that `casbin_rule` held 32 rows and nothing for `/system/games`.

**Cause.** `pkg/casbin`'s `cleanTable` deletes every row of `casbin_rule` to isolate each test, and
then restored a hardcoded copy of the rows that migrations 000008 and 000009 install. `casbin_rule`
holds BOTH the migrated policies and the throwaway rows a test writes, so that copy had to be
extended by hand for every later policy migration — and it was not extended for 000011, which seeded
`/system/games`. `migrator up` cannot repair the damage either, because a migration already recorded
as applied is never re-run.

**Why it cost time.** A 403 on every request reads as an authorizer or policy-shape bug, so the
search ran in the wrong place entirely. The status code was telling the truth; only the row count
was wrong.

**Fix.** `cleanTable` now reads the policy rows before clearing and writes that snapshot back in a
`t.Cleanup`. There is no second list to keep in step, so no later migration can be erased by a suite
run. `id` is deliberately not snapshotted — it is `GENERATED ALWAYS AS IDENTITY`, and nothing in the
adapter addresses a row by id.

**Verified.** 41 rows before and after a full `go test ./... -count=1`, with `/system/games` intact
at 8, and 10 packages passing.

**Residual, deliberately not fixed.** The table is still empty for the DURATION of each test, which
is what makes the concurrent-server hazard in B1 a mechanism rather than a suspicion.

---

## B9 — Garage was never provisioned, so object storage has never worked in dev

**Observed 2026-09-15.** The first upload of a unit class image failed with

```
AccessDenied: Forbidden: No such key: GKdevonlyreplacethis0000000000
```

**Cause.** `docker-compose.yaml` documents the provisioning steps in a comment — assign the node a
layout role, apply the layout, import the key, create the bucket, allow the key — and **none of them
had been run.** `garage status` reported the node as `NO ROLE ASSIGNED`, with no key and no bucket.
The steps are the ones written in the comment; the app's `AWS_ACCESS_KEY_ID` is a *fixed* id, so the
key must be **imported**, not created — a created key gets a random id and every upload stays refused.

**Why it matters beyond this feature.** Everything that touches object storage was non-functional:
the seeded default icons in `pkg/storage/object/seed.go`, and every presigned account photo, which
was a URL that could not resolve to an object. Nothing failed loudly because a presign is a local
computation — it produces a URL whether or not the bucket exists, and the 403 only appears when
something tries to actually move bytes. That is why this stayed invisible.

**Consequence for delivery.** A fresh deployment that skips these steps fails with an opaque S3 error
on the first upload. The steps belong in the deployment checklist, not only in a compose comment —
and the delivery bundle should ideally provision them itself, or fail at startup with a clear message
rather than at the first upload.

**Fixed.** Provisioned on 2026-09-15: node `c8a03303421c35aa` assigned to `dc1` at 10 GB, layout
applied, key `GKdevonlyreplacethis0000000000` imported as `minos-app`, bucket `minos` created and the
key granted RW. Verified end to end by a real upload, archive download and delete.

---

## B10 — Taxonomy DELETE contradicts G2, and the two records disagree

**Observed 2026-09-17.** `POST`/`PATCH`/`DELETE` were added for `/unit-categories` and
`/unit-types` (migration `000024`). **G2 asked exactly this and was answered the other way.**

Round 3's G2 was: *"Can a lookup row (a unit status, a category, a NATO type, a service branch) ever
be retired or deleted, or only renamed and added to?"* — categories and NATO types named in the
question. The answer, 2026-09-14:

> **G2 — lookup rows can only be renamed, never retired.** No `deleted_at` column is added to any
> lookup table, and none is needed. […] Lookup tables keep exactly one delete verb: **none**.

**The counter-evidence is one day newer and cuts the other way.** Migration `000019` (2026-09-15,
`unit_hierarchies`) describes the first lookup an operator authors at run time as *"renamed and
hard-deleted, never soft-deleted, because `is_system` already marks the rows the CMS must not offer
to delete"*. That table HAS a delete route. So the project's own record is internally inconsistent:
G2 says lookups are never deleted, and the next day's precedent for an operator-authored lookup
includes a hard delete.

**Why it matters more than a documentation tidy-up.** A DELETE the client never asked for is a
footgun with the safety catch missing, and the numbers are specific: `is_system` is a
CLIENT-side signal only (see B11), the foreign key protects a type only when a class points at it,
and **18 of the 19 seeded unit types have no class** — so 18 of them are deletable by any caller
holding the `delete` action. A misclick removes a NATO type the fleet's whole vocabulary is built
from, with no tombstone and no undo.

**DECIDED 2026-09-21 — RETIRE, WITH A CASCADE, and G2 is deliberately reversed.**

A scope update settled this. The operator is expected to interact with `unit_categories`,
`unit_types`, `unit_classes` and `units` — all four levels — and that is what the delete routes were
asked for. On that reading G2's "lookup tables keep exactly one delete verb: none" was answering a
question about REFERENCE data that nobody edits, which is no longer the shape of the feature.

So the verb becomes **retire**, and retiring a parent retires everything beneath it. The frontend owns
the confirmation, and the cascade is intended rather than a hazard to be prevented.

**What the four levels look like today**, which is most of the work already done:

| level | tombstone | `is_system` | current DELETE |
|---|---|---|---|
| `unit_categories` | **no** | yes | hard, refused by the FK when types point at it |
| `unit_types` | **no** | yes | hard, refused by the FK when classes point at it |
| `unit_classes` | **yes** | no | already a retire |
| `units` | **yes** | no | already a retire |

So the lower two levels already retire, and the upper two are the ones that change: they gain
`deleted_at`/`deleted_by` and stop being hard deletions. After that all four behave the same way, and
a cascade is a tombstone written at each level rather than a chain of statements the foreign keys
refuse.

**HARD DELETION IS NOT AVAILABLE for a cascade anyway**, which is worth recording because it settles
the tombstone question rather than leaving it to preference. Every taxonomy foreign key is NO ACTION:
`unit_types.id_unit_category`, `unit_classes.id_unit_type` and `units.id_unit_class` all refuse, and a
unit is further held by `game_units`, `unit_spec_versions`, `unit_commanders`, `unit_position_log` and
`unit_positions_latest`. A hull that has ever been in an exercise cannot be deleted at all. Only
`service_branch_unit_categories` cascades.

**`is_system` does NOT block a retire**, and that is a deliberate difference from `game_roles`. All 10
categories and all 19 types are seeded, so refusing seeded rows would make the feature do nothing.
The reason the trade differs: retiring is REVERSIBLE — it is a tombstone, not the removal of a row a
policy points at — so the argument that made `GameRoleService.Delete` refuse (deleting `Game Master`
leaves the transition policy naming a row that no longer exists) has no counterpart here.

**What the cascade actually reaches, which is the number worth seeing before building it.** The dev
register holds 124 hulls across 10 categories:

| retiring | hulls retired with it |
|---|---|
| Amphibious | 25 |
| Patrol Craft | 22 |
| Fast Attack Craft | 21 |
| Corvette | 18 |
| Frigate | 13 |
| Auxiliary | 12 |
| Survey | 5 |
| Submarine | 4 |
| Mine Warfare / Training | 2 each |

**BOTH QUESTIONS ANSWERED, AND THE FIRST SLICE IS BUILT — 2026-09-21.**

**1. A hull inside a live exercise → report it, and refuse only in `execution`.** The frontend owns
the confirmation, so it has to be TOLD what it is confirming: the counts come back in the response,
and a preview route (`GET /unit-categories/{id}/retirement`, same shape at all four levels) returns
them BEFORE the act rather than after it. A count that only appears in the response to the retire is
no use to a dialog, which is the whole reason the preview exists.

The refusal is deliberately narrow — **`execution` only**, not every un-closed game. Retiring is
irreversible through the API (there is no un-retire; recovering 25 hulls means SQL), so a refusal has
to exist somewhere. But `planning` and `closure` are normal times to restructure a taxonomy, and
refusing those would trap an operator behind a game nobody is using. In `execution` the damage is
different in kind: `game_units` is NO ACTION, so the assignment survives, while `UnitRepo.List`
filters `deleted_at IS NULL` — the hull vanishes from the exercise's order of battle mid-exercise,
with the command centre still holding a row nobody can see.

**2. The route → dedicated retirement endpoints, and `delete` leaves the taxonomy.** All four levels
get `POST /{resource}/{id}/retirement`, matching `PUT /games/{id}/readiness` and
`POST /games/{id}/transitions`. A cascade that can withdraw 25 hulls is not what `DELETE` on one
resource reads as. The four DELETE routes and their four grants come out.

**`000029_unit_retirement` is built and verified** — up → down → up, plus a from-scratch build of all
29 migrations into an empty database. It adds `deleted_at`/`deleted_by` to `unit_categories` and
`unit_types`, and two things it turned up are worth keeping:

**The uniqueness rules are CONSTRAINTS, not indexes.** `unit_categories_name_key` and
`unit_types_name_key` are `UNIQUE` constraints from the original CREATE TABLE — `contype = 'u'` —
whereas the `uq_*_active` indexes on `unit_classes` and `units` are bare indexes. The two look
identical in `pg_indexes`, and `DROP INDEX` on a constraint-backed one fails with a hint telling you
to drop the constraint instead, which is how this was found. Both are replaced by partial indexes,
and the down restores them as constraints so the schema genuinely returns to where it started.

**`retire` is a new ACTION rather than a reuse of `delete`** (`casbin.ActionRetire`), on the same
grounds `ActionTransition` has one. The `delete` grants on the four objects are revoked in the same
migration, and that was checked rather than assumed: `delete` on `/system/units` and
`/system/unit-classes` is asked for by nothing but the DELETE routes being removed — 000013's image
archive reuses `update`.

**`is_system` is now vestigial on the taxonomy, and that is a client decision rather than a code
one.** G2 says a category may be RENAMED and this entry says a seeded row must stay RETIRABLE. Those
are the two affordances the flag exists to let the CMS disable, and both are now decided against it.
But the COLUMN cannot simply go: `helperColumns` is one list across all ten lookups, so it must exist
on every table in `HelperTables`; and `HelperPublic` is ONE struct embedded in the unit, class, type,
branch and game-role payloads, so a per-table value would have the same field name meaning different
things inside a single response. See B11.

**Deliberately NOT moved out of `HelperTables`.** The taxonomy stays in the shared lookup read,
because `/helpers` is where a client resolves `unit_categories` and `unit_types` ids from — the
contract says so in about twenty places. `service_branches` must stay for a blunter reason: there is
no `/service-branches` list route at all, so `/helpers` is its ONLY enumeration surface, and removing
it would leave clients unable to render the Matra picker. The isolation that was actually wanted is
got instead by teaching the shared read a per-table tombstone predicate.

**Built ahead of the cascade, since both are prerequisites:** the tombstone migration above, and
`models.HelperTable.Retires` — the shared lookup read now excludes tombstoned rows for the two
lookups that retire and is untouched for the other eight. `TestEveryRetiringHelperHasATombstone` reads
`information_schema` and asserts that list agrees with the schema in both directions;
mutation-checked by listing `service_branches`, which fails naming the table and the fix.

**BUILT AND VERIFIED 2026-09-21.** The cascade, the four `POST /{resource}/{id}/retirement` routes,
the `execution` guard, the OpenAPI changes and the tests all shipped; the four `DELETE` routes and
their grants are gone, replaced by `retire`. `000029` was verified up → down → up **and** against a
from-scratch build of all 29 migrations into an empty database.

Three things the work turned up, recorded because each would have been found the hard way:

- **The cascade must run BOTTOM-UP**, and that is mutation-checked rather than asserted. Reversing
  the order so types are tombstoned first makes the classes and hulls match nothing — the test
  reported `Classes:0 Units:0` for a category holding one of each, while the category itself still
  read as retired. Quiet enough to need a test instead of a code reading.
- **`game_units.id_commander` is protected by a composite foreign key** into `game_participants`.
  The test fixture could not assign a hull until its commander was a participant. That existing
  constraint is why the guard can join straight to `games` without asking who commands the hull.
- **The taxonomy's uniqueness rules are `UNIQUE` CONSTRAINTS, not bare indexes** — `contype = 'u'` —
  where the `uq_*_active` rules on `unit_classes` and `units` are bare indexes. They look identical
  in `pg_indexes`, and `DROP INDEX` on a constraint-backed one fails with a hint telling you to drop
  the constraint.

**One piece was deliberately dropped: the preview route.** It was planned so a client could see the
blast radius BEFORE confirming, since the confirmation lives in the frontend. The operation ships
without it because the counts come back in the response — but it does mean the confirmation dialog
cannot yet say "25 hulls, 3 of them in a running exercise". That is the next step here, not a defect.

---

## B11 — `is_system` is enforced by the client and nothing else

**Observed 2026-09-17.** Every seeded category and unit type carries `is_system = true`, and
`HelperPublic.IsSystem` is documented as existing *"so the CMS can disable editing and deleting of
rows the system owns"*. The server does not read it on any write path.

This matches `HierarchyService.Delete`, which relies on the foreign key and leaves the rest to a
disabled button. It was confirmed deliberately for the taxonomy on 2026-09-17 rather than
overlooked.

**Why it is worth a line.** A disabled button is not a rule. The contract now says so in the
operation descriptions, and the CMS teams have to implement it — but if they do not, nothing fails:
the delete succeeds. The exposure is quantified in B10.

**Next step.** Confirm with the frontend team that they disable the delete affordance on
`is_system` rows. If that cannot be relied on, option 3 in B10 is the server-side answer.

**Partly closed 2026-09-18, for `game_roles` only.** `GameRoleService.Delete` now refuses a
system-owned row with a `409` before issuing the statement. The reason the trade that was accepted
here is not available there is that the consequence is not comparable: the seeded game roles are
**named by the policies that grant in-game authority**. Migration `000026` grants the state
transition to `game_role:<id of Game Master>`, so deleting that row leaves a policy pointing at a
role that no longer exists — and the outcome is that no exercise can ever leave planning again, with
nothing in the API to say why. Losing a seeded category that nothing points at is a tidier problem.

**The taxonomy still follows the original arrangement**, deliberately and unchanged. Making the two
consistent is a decision to take rather than a gap to close — the taxonomy tables have no policy
naming them, so the argument above does not carry over, and the server-side guard there would be a
rule invented for a risk nobody has demonstrated.

**It is not a general fix for this item.** `service_branches`, `user_statuses`, `movement_domains`,
`app_roles` and `deployment_statuses` remain unwritable, so the question does not arise for them yet;
and `unit_hierarchies`, which IS writable, has no guard at all.

**Updated 2026-09-21 — the question is now specific, and it is with the frontends.** On the taxonomy
the flag has no remaining job: G2 says a category may be renamed, and B10 says a seeded row must stay
retirable — the two affordances it exists to let the CMS disable, both now decided against it.
`service_branches` is the one to leave alone: it has no audit columns at all, so `is_system` is its
only trace of having come from a migration, whereas the two taxonomy tables record it properly as
`created_by = 1` (000024's backfill) — which is 000019's own argument, that a record beats a marker.

**Dropping the column is not available; the answer is to stop EXPOSING it.** `helperColumns` is one
list across all ten lookups, so the column has to exist on every table in `HelperTables`; and
`HelperPublic` is one struct embedded in the unit, class, type, branch and game-role payloads, so
dropping it from three tables would leave the same field name meaning two different things inside a
single response. Removing `HelperPublic.IsSystem` instead is one field in one struct, uniform across
every embed, and it is a *visible* break — a generated client loses a field — rather than a silent
value flip from `true` to `false`. Raised as a question for the CMS teams rather than decided
unilaterally. `unit_hierarchies` and `game_roles` keep both the column and their server-side checks.

---

## B12 — Lookup ordering sorts by a label nobody displays, and depends on the image's libc

**Observed 2026-09-17.** `HelperRepo.List` is `ORDER BY name ASC` for every lookup, and on the dev
database the comparison is byte order, so it is **case-sensitive**:

```
Auxiliary | Submarine | frigate | survey
```

Two consequences, neither of them cosmetic for the taxonomy screen.

**It sorts by `name`, and the screen draws `id_name`.** For categories `name` is the English label
("Frigate") and for types it is the NATO symbol ("SSK"). The Indonesian labels an operator actually
reads are therefore NOT in alphabetical order in the language they are read in.

**A lowercase name sorts after every capitalised one.** Nothing enforces capitalisation, and as of
`000024` an operator can create a category — so typing `survey` instead of `Survey` files the new
row at the bottom of the list, where it reads as having vanished.

**The mechanism is the image's libc, which makes this a B1-class environment dependency.** The
database runs `postgis/postgis:16-3.5-alpine`; Alpine uses musl, whose locale support is minimal, so
`en_US.utf8` degrades to byte comparison. As long as the offline bundle ships the same image, dev and
the exercise LAN agree. **If it ever ships a glibc-based Postgres, the list order changes between
environments** — see D1, still open, which asks for the target OS *"because the offline Docker bundle
must be built for the matching distro/libc"*.

**Not fixed, deliberately.** The fix is `ORDER BY lower(name), name` (or an explicit `COLLATE "C"`),
which is one line in the shared lookup read — and that read backs **all ten lookups** and every
payload that embeds one, so the ordering of other screens would change with it. That is a decision
for the client, raised as I3 in `client-questions.md`.

**Corrected 2026-09-21 — the count above was wrong, and there is now a second case inconsistency.**
`models.HelperTables` holds **ten** entries, not eleven, and has since `000021` added
`deployment_statuses`. B5 said "eight" for the same reason in the other direction. Both are corrected;
see B5 for the full note.

The new one is a smaller version of the same problem, and `000029` created it deliberately. The two
upper taxonomy levels were given partial unique indexes in that migration which PRESERVE the rule they
already had, and the rule is not the one the lower two levels use:

```
unit_categories   uq_unit_categories_name_active   ON (name)          WHERE deleted_at IS NULL
unit_types        uq_unit_types_name_active        ON (name)          WHERE deleted_at IS NULL
unit_classes      uq_unit_classes_name_active      ON (lower(name))   WHERE deleted_at IS NULL
units             uq_units_name_active             ON (lower(name))   WHERE deleted_at IS NULL
```

So `Frigate` and `frigate` can coexist as categories and cannot as classes, and an operator who types
`frigate` gets a different answer from the two screens. **000029 deliberately did not change it** —
changing a uniqueness rule is not what a tombstone migration is for, and folding a silent behaviour
change into it would hide one decision inside another. **It should still be decided, and the lower two
levels are the ones that are right**, because nothing enforces capitalisation at either level and this
entry already records what that costs: a lowercase name sorts after every capitalised one, where it
reads as having vanished. It is also the same underlying question as I3, and answering one without
the other leaves the vocabulary half-normalised.

---

## B13 — The game domain was built but dormant; it is now live (partly closed 2026-09-18)

**Observed and fixed 2026-09-18.** `pkg/casbin` has defined the domain-relative game objects
(`/games/units`, `/games/judgements`, `/games/control`, …) and `GameDomain(gameID)` since the start,
and `adapter_test.go` exercises the whole mechanism — a Referee reads units in their own game, is
denied in another game, and cannot steer a unit.

**None of it was reachable.** `GameDomain` had **no production caller**, no migration had ever seeded
a game-role policy, and `casbin.AddRole`/`RemoveRole` were only ever called with `SystemDomain`. Every
game route was gated by `/system/games` in the system domain, so in-game authority was decided in
`GameService.Transition` by comparing a role **NAME**.

That mattered because the client's answer G2 is that lookup rows may be **renamed**, and game-role
authoring makes renaming reachable.

**Closed.** Migration `000026_game_role_policies` seeds the single policy that reproduces the check it
replaces (`Game Master` → transition), the service now writes game-scoped grouping rules on every
roster change and revokes on removal and game deletion, `SyncGameRolesFromDatabase` reconciles at
startup, and `Transition` authorises through the policy.

**The subject is the role ID, not its name.** This is the load-bearing decision. Replacing a Go name
comparison with a policy that NAMED the role would have moved the fragility into the policy store
rather than removing it: the matcher is `g(r.sub, p.sub, r.dom)`, so `p, Game Master, …` stops
matching the moment the lookup row is renamed, for every holder at once. Verified live:

| step | grouping rule | transition |
|------|---------------|-----------|
| participant added as Game Master | `user:2 game_role:2 game:647` | 200 |
| demoted to Commando | `user:2 game_role:1 game:647` | **403** |
| promoted back | `user:2 game_role:2 game:647` | 409 (readiness gate, so authority passed) |
| role renamed to `Direktur` | `user:2 game_role:2 game:647` — **unchanged** | 409 (still authorised) |

**CLOSED 2026-09-21 — the second name check is gone too.**

`participantIsStaff` used to read `participant.Role.IsJudgeSide || participant.Role.Name ==
casbin.RoleGameMaster`. It now takes a `runsTheExercise` fact supplied by the caller, and the callers
ask `GameAuthority.RunsTheExercise` — which is **the same question the state machine asks**, so driving
an exercise and seeing it cannot disagree.

**Why that option and not the other two.** Three were available: key the check on the role **ID**, add
a boolean column to the lookup row, or derive it from the policy. The ID would have hardcoded 000010's
seed order and, worse, would have DIVERGED from the matrix — grant `transition` to a Deputy and the
Deputy could run the exercise but would not be able to see it. A column would have been a second
record of the same authorisation fact, which is exactly what B16 says not to keep. Deriving it is the
only one that cannot go out of step.

The **judge half stays on the column**, and that is not inconsistency: `is_judge_side` is not derived
from any policy. It is the INPUT that decides who the readiness gate asks, and 000012 made it a column
precisely so a rename could not break it.

`gameVisibility` gained a third parameter rather than a receiver or a lookup. It stays pure, which is
why two services can share one definition of the rule; the fact is threaded in by the four call sites,
and `gameAccess.RunsTheExercise` carries it on the game read.

`GameAuthority.MayTransition` was renamed `RunsTheExercise` as part of this, because one method now
answers both callers and a second name for the same question is the drift this fix exists to prevent.

**Verified live.** The exact case that failed before, on a game with an area, in planning:

| step | area in the payload |
|---|---|
| role named `Game Master`, holds the transition | `'Laut Jawa'` |
| renamed to `Direktur` | **`'Laut Jawa'`** — was `null` before the fix |
| after an API restart | `'Laut Jawa'` |

**And the tests pin both directions.** The table now carries a case that would have failed under the
previous rule — *"a role RENAMED away from 'Game Master' that runs the exercise"* — and a stronger one
that would also have failed: *"a role still CALLED 'Game Master' that does not run the exercise"*, which
must be ejected at closure like any other participant. Mutation-checked by restoring the name
comparison: both fail, with the second reporting `Entitled = true, want false`.

**Also closed:** `TestGameRoleNamesMatchTheLookup` was named in `pkg/casbin/constants.go` as the
mechanism keeping the role constants and 000010's seed in step, and **it did not exist**. The comment
said "a comment asking a future reader to keep two lists in sync is not a mechanism; a test is" and
was itself the only mechanism. Written now as `TestTheGameRoleVocabularyMatchesItsSeed`, which also
covers the name `000026` resolves its subject from.

**The matrix is authorable as of 2026-09-18** — see B14. `game_roles` itself is authorable, with one
deliberate difference from the taxonomy recorded under B11.

---

## B14 — Participants cannot reach the in-game routes, so the matrix governs less than it appears to

**Observed 2026-09-18, while making the permission matrix authorable.** The game domain is live: a
roster change writes a game-scoped grouping rule, and in-game authority is decided by policies keyed
on the role id. But **every route under `/games/{id}` is still gated by `/system/games` in the SYSTEM
domain** — `administer(action)` in `internal/routers/game.go`.

So the matrix decides authority WITHIN an endpoint the caller can already reach, and nothing more.
A Referee can be granted `/games/judgements` `judge`, and the policy will evaluate correctly — but if
the Referee is not an application administrator, they cannot call the endpoint that checks it.

**Why this was not fixed in the same slice.** It is not a wiring change. Each in-game route needs its
object and action mapped, the route-level group gate replaced with a game-domain resolver, and the
row-level checks preserved — the ones that currently say "you must be this game's participant", which
no policy can express because an object carries no instance id. `docs/game-modes.md` §2 states the
intended shape: *"Authorization is game-scoped through the existing Casbin domain, with per-object
`own` rules plus a service-level instance check where a policy cannot reach."*

The route comments already anticipate it — `POST /games/join` and the readiness pair are
authentication-only with an instance check in the service, and the comment explains that Echo cannot
express "this permission OR that participation" as a chain.

**The blast radius today** is that the in-game half of the API is staff-only. That is safe rather
than broken, and it is why `ActionOwn` exists in the vocabulary but is asked for by no route: the
per-rules it was meant for are part of the same unwritten slice.

**Next step.** Decide with the client whether participants use the API directly or through the
Centrifugo proxy only. If they use it directly, this is the change that makes the feature reachable —
and it should be done route by route, because each one needs its instance check re-examined rather
than inherited.

### Closed 2026-09-21 — `RequireGamePermission`

Every route under `/games/{id}/...` now admits **two kinds of caller**, and the gate asks both
questions in one middleware:

1. the action on `/system/games` in the **system** domain — what `administer` already did, so nothing
   that worked before stopped working;
2. the action on a **game object** in that game's **own** domain — the permission matrix.

Either is sufficient. The administrator path is asked FIRST, which is deliberately the opposite order
from `gameAccessFor`; the two are not in conflict and the middleware's header says why. `gameAccessFor`
puts the participant first so an administrator PLAYING A COMMANDO cannot read the area early — that is
about what they may SEE, and seeing less is the safe direction. Refusing a request because a second,
narrower role the caller also holds does not permit it would remove a capability the application
permission already grants, and would break the CMS for any operator who happens to be in the game.

Two things the slice turned up:

**`/games/hierarchy` did not exist as an object.** The task organisation had no object in the
vocabulary, because until now no route needed one — arranging the tree was covered by the application
permission that gated the route. Added, and it is a resource of its own rather than part of
`/games/units`: a Commando steers their own hull and has no business reorganising the tree above them.

**`games` was used as an operation tag but never declared.** The OpenAPI listed four tags and the
game operations referenced a fifth, so a generated client had a tag with no description. Declared now,
with the two-path rule written out once and each affected operation pointing at it.

**Not widened, deliberately:** `GET /games` and `POST`/`PATCH`/`DELETE /games/{id}` stay application
capabilities. Creating, editing and withdrawing an exercise is a CMS operation, and the in-game roles
govern what happens *inside* one. `POST /games/join`, the readiness pair and `POST /games/{id}/transitions`
stay authentication-only with an instance check in the service.

**Verified live** against a participant holding NO application role at all, which is what makes the
test meaningful — with an administrator, the first path would answer every time:

| probe | result |
|---|---|
| participant, no grant, own game | **403** |
| application admin, not a participant | 200 — the regression guard |
| participant granted `/games/units` read, own game | **200** |
| the same grant, a different game | **403** — grants do not cross games |
| a route the role holds nothing for | **403** |
| after revoking the grant | **403** |

**Still open, and it is a product question rather than a defect:** whether participants are expected to
use these routes directly, or only through the Centrifugo proxy. The plumbing now supports either; the
answer decides whether the remaining per-object rules (`own`, and the instance checks the route
comments describe) need writing.

---

## B15 — A game role's permission matrix is invisible to the reconciliation's second half

**Recorded 2026-09-18 as a known limit rather than a defect.** `SyncGameRolePermissionsFromDatabase`
sweeps policies whose SUBJECT is a game role AND whose object is a game object. A policy that fell
outside either filter is left in place, deliberately, on the grounds that visible-and-unmanaged is a
failure a human can find.

The consequence is that a hand-written `casbin_rule` INSERT claiming a game role holds
`/system/users` would survive a restart: the sweep would log a warning and step over it. It cannot be
reached through the API — `IsGameObject` refuses it, and the `CHECK` on the table refuses it even if
the service were bypassed — so it requires direct database access, which already implies the ability
to grant anything.

Worth knowing so that the warning line is not read as noise when it appears.

---

## B16 — A game-role permission granted by a migration must be a ROW, not a policy

**Recorded 2026-09-18, from a hazard found while writing 000028.** Not a defect and not pending work —
a convention the next person to add a game-role permission has to follow, written down because
breaking it fails in the quietest way this codebase has produced so far.

`game_role_permissions` is the source of truth, and `SyncGameRolePermissionsFromDatabase` reconciles
`casbin_rule` to it **at startup, in both directions**. The revoking direction is what makes the
matrix work — it is how a withdrawn permission stops being honoured — and it is also what turns a
directly-written policy into an orphan.

- 000026 seeds the transition by INSERTing into `casbin_rule` directly. Before 000028 that is
  self-contained; after it, the row is in the store with nothing in the table backing it.
- With the table empty, the reconcile finds it in `current`, finds no `desired` entry, and calls
  `RemovePolicy`. `game_role:2` loses the transition, `MayTransition` returns false for everybody
  including the Game Master, and every `POST /games/{id}/transitions` answers **403** — permanently.
- The only trace is `casbin: game role permissions reconciled against game_role_permissions ...
  revoked=1`, which reads exactly like a routine reconciliation.

**Why it would not have been caught.** Migrating and serving are different containers. The `migrator`
applies 000026 cleanly and exits 0; the sweep happens on the next `api` start, minutes later, in a
different log stream.

**The rule.** A migration written after 000028 writes a row and nothing else, because the policy is
derived from it:

```sql
INSERT INTO game_role_permissions (id_game_role, object, action, created_by)
SELECT r.id, '/games/judgements', 'judge', 1 FROM game_roles r WHERE r.name = 'Referee';
```

000028's backfill is what bridges 000026's policy-only seed. It is also stated at the top of both
files, because that is where somebody copying the pattern will actually be looking.

**Related, from the same migration.** 000028 introduces a STARTUP dependency on its own table: the
API reconciles against `game_role_permissions` before it serves anything. Rolling the table back while
running a binary that expects it leaves the process unable to boot, with an error naming a relation
rather than a version mismatch. Rolling the image back too is the whole of the fix, and it only
surprises someone applying SQL by hand to a development database.

---

## B17 — Submarines cannot be ordered to move submerged, because the fleet has no submerged speed

**Observed 2026-09-21, while building the movement clamp.** `speed_max_submerged_kn` is **NULL for
every submarine in the fleet**. The import of `daftar-aset-kapal-TNI-AL-v2.xlsx` recorded the
**surfaced** maximum as `speed_max_surface_kn` (11 knots for a Cakra-class boat) and left the
submerged column empty, noting in `spec` that the sheet gives the surfaced figure only.

**Why that is a refusal rather than an unclamped order.** An order's speed is clamped to the hull's
specification maximum for the movement domain it is in — `Surface` reads `speed_max_surface_kn`,
`Submerged` reads `speed_max_submerged_kn`. When that column is NULL the maximum is *unknown*, and
`models.HullSpeedLimits.MaxKn` **fails closed**: an order that cannot be clamped is refused with
409 rather than let through unbounded. That is deliberate and is the whole point of the clamp — a
client with a slider is not trusted, and "we could not bound this" is not a reason to accept it.

**So the present behaviour is:** any attempt to order a submarine to move submerged is **refused**,
with a message telling the Game Master to publish a specification with a maximum speed for the
domain the hull moves in. The boat is unusable in its own element, and the symptom looks like a bug
in the order path rather than a gap in the data.

**The options, none of which is obviously right.**

1. **Publish submerged speeds.** Correct, and the only one that makes the clamp meaningful for
   submarines. Needs a source: the sheet does not have them, so this is research — probably public
   figures per class (Cakra/Chang Bogo are commonly cited around 21.5 knots submerged).
2. **Fall back to the surfaced maximum when the submerged one is absent.** Small change, unblocks
   the boats immediately, and is **wrong in the direction that matters**: it would cap a submarine at
   11 knots under water when it can do roughly twice that, so every submerged leg would be silently
   too slow. Worse than a refusal, because a refusal is visible.
3. **Treat a missing maximum as unbounded for that domain only.** Rejects the premise of the clamp
   for the hulls least able to be checked by eye. Not recommended.
4. **Add a per-hull "unclamped" flag** an administrator can set with a reason. Preserves fail-closed
   as the default while giving somebody a way to unblock a boat deliberately, and leaves an audit
   trail. More surface area than option 1, but it also covers the next hull whose data is missing.

**Needs a decision from the team**, not from the code: this is a data question about the fleet, and
options 1 and 4 differ in who is responsible for accuracy. Option 2 is listed only so that it is
recorded as considered — it should not be taken.

**Also worth confirming while it is being discussed:** whether `speed_max_surface_kn` for submarines
means *surfaced* or *snorkelling*, since the clamp will use it for every surfaced order. The import
note says surfaced, but it was read off a spreadsheet column rather than a specification.

**Until then, nothing is broken for surface hulls** — every one of them has a surface maximum, which
is the domain they move in.

---

## B18 — A refused `DELETE /games/{id}` is explained in the words of a `PATCH`

**Observed 2026-09-22**, rehearsing the Execution phase end to end over HTTP
(`scripts/rehearse_game.py --cleanup`).

**What happens.** Deleting a game that has left `planning` is refused with 409, and the body says:

    "_request": "A game's terms may only be changed while it is in `planning`. This one is in `execution`."

**The refusal is right.** `Delete` and `UpdateByID` share `loadMutable`, and sharing it is deliberate
— the guard asserts "a game past planning is immutable", which is as true of a delete as of an edit,
and the comment on `loadMutable` records the reasoning ("a comparison written twice is one written
differently once"). A game's area and its assumed start are things other records were built on, so
withdrawing the game would orphan them. Nothing here should be relaxed.

**The message is wrong, though.** A `DELETE` is not a `PATCH`, and the sentence names the wrong act.
An operator who tried to remove an exercise is told its **terms** cannot be **changed**, which reads
as a complaint about an edit they never attempted, and sends them looking for the `PATCH` that did
it. The 409 is correct; the explanation is about a different request.

**The fix has one wrinkle.** The obvious change is to pass the act into `loadMutable` so it can say
"may only be **deleted**" or "may only be **changed**". But the two callers want the same RULE with
different verbs, so the parameter is the only thing that differs — and a third caller that forgot to
pass one would silently get whichever wording was the default. The alternative is a message that
names neither act and describes the rule instead:

    "A game past `planning` is fixed: its force, its area and its assumed start are what the
     records built on it depend on. This one is in `execution`."

**Recorded rather than fixed before delivery** — it is cosmetic, and the rule it reports is correct.
It is here because it is exactly the kind of thing a reviewer notices in a demo, and because it is
the only refusal found so far that explains an action the caller did not take.

**Related, and NOT a defect:** a game past planning cannot be deleted at all, so a rehearsal or a
demo leaves its exercise behind for good. That is the same window working as intended — see §6.1 of
`docs/game-modes.md` for why a game's terms are frozen — and it is why
`scripts/rehearse_game.py --cleanup` reports a refusal rather than a deletion.

---

## B19 — Most of the in-game permission matrix is vocabulary for features that do not exist yet

**Recorded 2026-09-22, as a scope decision rather than a defect.** B13 and B14 made the matrix
authorable, reachable and enforced; this entry records how much of it anything actually asks for, so
that the question a reviewer will ask — *"you built a permission matrix; what does it govern?"* — has
a written answer rather than a demonstration that happens to be done twice and described differently.

**The inventory, taken from the code rather than from the vocabulary's own documentation** (production
files only; every count below excludes `constants.go` and every `_test.go`):

| asked for by | object | action | where |
|---|---|---|---|
| a route middleware | `/games/participants` | `read`, `update` | `inGame(...)` in `internal/routers/game.go` |
| a route middleware | `/games/units` | `read`, `update` | same |
| a route middleware | `/games/hierarchy` | `read`, `update` | same |
| the service, not a route | `/games/transitions` | `transition` | `GameAuthority.RunsTheExercise` |

| declared, and asked for by **nothing** | |
|---|---|
| objects | `/games/messages`, `/games/judgements`, `/games/timeline`, `/games/review`, `/games/control`, `/games/*` |
| actions | `order`, `judge`, `send_message`, `pause`, `change_factor`, `warn`, `own` |

**Three of those seven actions have live routes that deliberately do not use them.** That is the part
most likely to be mistaken for an oversight, and it is the opposite: `POST .../order`, `POST .../pause`
and `PATCH .../time-factor` are **authentication-only, with the authority check in the service**,
because the rule is *"do you command THIS hull"* or *"do you run THIS exercise"* — an instance
comparison over a row. A Casbin object names a resource FAMILY and carries no instance id, so no
policy could express it, and a route gated on `ActionOrder` would refuse every Commando. The reasoning
is already written at each of those routes; `ActionOrder` and `ObjectGameControl` appear in the
codebase **only inside those comments**, which is why a grep that does not separate comments from code
reports them as used.

**The state, 2026-09-22: the matrix governs the setup and lifecycle objects only** — `participants`,
`units`, `hierarchy` and `transitions`. **The rest is NOT "closure-phase vocabulary", which is what
this entry said when it was first written and was simply wrong.** `judge`, `send_message` and `warn`,
with `/games/judgements`, `/games/messages` and `/games/control`, belong to **EXECUTION** — concept §3
items 6, 3 and 8 — so they are unfinished work in the phase being delivered, not vocabulary reserved
for a later one. Only `/games/timeline` and `/games/review` belong to Closure.

That reassignment is the point of this correction and it is a scope question, not a tidiness one: it
moves four items from "future phase" into "unbuilt requirements of the current one". See
`docs/game-modes.md` for the phase split and `docs/concept-en.txt` §3 for the list itself.

So the matrix is not dead code and not a stub: it is authorable, reconciled against
`game_role_permissions` in both directions at startup (000028), and enforced on the four objects that
have callers.

**Why that is defensible rather than a gap.** The matrix's remaining entries have no consumer because
the FEATURES they belong to do not exist yet — no `game_judgements` table, no message table, no
`/games/messages` path in the contract. Shipping the vocabulary ahead of the code is what let
`/games/hierarchy` be added as an object in one line when the tree needed one (B14), and it is how the
`transition` policy could replace a role-NAME comparison without inventing anything (B13).

**CLOSED 2026-09-22 — a grant nothing asks for is now REFUSED, and the vocabulary is checked against
the routes rather than against itself.**

The trap was live. `PUT /game-roles/{id}/permissions` validated the OBJECT against `GameObjects` and
the ACTION against `GameActions` — two lists that each answer half the question, and whose cross
product is a hundred pairs of which seven are real. So `/games/transitions` + `read` satisfied both
checks and was exactly as inert as `judge`.

`casbin.GamePermissionsEnforced` now names the pairs something actually asks for — `participants`,
`units` and `hierarchy` with `read` and `update`, plus `transitions` with `transition` — and the write
refuses anything outside it with a 400 naming the field. `GameObjects` and `GameActions` keep their
real jobs: the first is the NAMESPACE the reconciliation sweeps, the second is what a game role may
ever name. Both comments were corrected, because both claimed a criterion their contents did not meet.

**The old guard was the interesting part.** `pkg/casbin` carried `TestGameActionsExcludesCreateAndDelete`,
whose second loop asserted that `GameActions` contained a hardcoded list of ten actions, reporting
"IsGameAction(%q) = false, but a game route checks it" — which was FALSE for seven of them. It checked
the list against a copy of itself, so it passed while the list was wrong: precisely the failure
`scripts/verify_deployable_version.sh` describes in its own header. It is replaced by
`internal/routers/game_permission_vocabulary_test.go`, which reads the ROUTER SOURCE, strips the
comments out — stripping them is the point, because the misleading evidence lived in exactly those
comments — resolves the constant names by reading `pkg/casbin/constants.go`, and fails if the two sets
disagree in either direction. Mutation-checked by swapping one real pair for a dead one: both
directions fire with the permission named.

**Verified live**, against a rebuilt binary: `judge` on `/games/judgements`, `read` on
`/games/transitions` and `read` on `/games/messages` are each refused 400 with a message saying nothing
asks for them; `/system/users` still gets the SECURITY message and `evaluate` the ACTION message, so
the order of the checks still tells an operator which mistake they made; and real pairs are accepted.

**What this does NOT do is make the matrix govern more.** It makes it honest about what it governs —
the table above is the whole of it. The unbuilt parts of **Execution** (messaging, judgements and
warnings) and Closure's review reads are what would give the rest of the vocabulary a caller.

**What would reopen this:** building those Execution features. `judge`, `send_message` and `warn` are
what such a change would ask for, and `/games/judgements`, `/games/messages` and `/games/control` are
what it would give a caller to. `/games/timeline` and `/games/review` are Closure's and would arrive
with that phase instead.

---

## B20 — Execution is four items short, and three of them are deferred by decision

**Recorded 2026-09-22.** Concept §3 defines the Execution phase as **eight** things. Four exist —
maneuvering, the measure queries, the time factor and pause — and four do not: **messages**,
**judgements**, **simulated events** and **warnings**. There is no table, no route and no contract
path for any of them, and this entry records what is being done about each so that four unbuilt
requirements do not read as one undifferentiated gap.

| §3 | item | disposition (2026-09-22) |
|---|---|---|
| 3 | messages (Telegram + administrative) | **IN SCOPE** — being scoped, then built |
| 6 | judgements | **SKIPPED for now** |
| 7 | simulated events | **DEFERRED to the backlog** |
| 8 | warnings | deferred alongside judgements |

**Simulated events is the least specified thing in the phase**, which is why it is the one deferred
rather than merely skipped. The concept gives it one line — *"able to 'Simulate' random events during
this Phase (Natural Disasters, Synthetic Disasters, Orders from the Higher Up, etc)"* — and there is
**no client question about it anywhere** in `docs/client-questions.md`. Not what a simulated event
IS (a message? a map marker? a change to the scenario clock? a forced order?), not whether the players
are told or only the Game Master, not whether Closure replays it, not whether it can move a hull.
Building it would mean inventing four answers and presenting them as requirements, which is how a
product acquires features nobody asked for.

What IS clear is that it overlaps **messaging**: "Orders from the Higher Up" reaches the players as a
message, and the Scenario Role from Planning item 12 exists precisely to send a Telegram message *as*
a higher authority. So the messaging build should be shaped to make a simulated event expressible
later — a message with an assumed identity and a classification is most of one already — without
building the event mechanics.

**Judgements and warnings are a pair, and both are already designed.** Client answers pin the parts
that matter: judgements are **GM and Judges only** and never reach unit or node Commandos (B8), the
score is **free text for this release** (B9), and warning codes are a **lookup table the client
populates** (B7). The substrate exists as well — the concept wants a judgement against "every action
taken", and `game_fixes` already records every action, so a judgement attaches to a leg rather than
requiring a new notion of "action".

**§3 items 4 and 5 are built, but NARROWER THAN THE CONCEPT.** It says *"Game Master **and Judges**"*
can pause and change the time factor; the clock routes ask `GameAuthority.RunsTheExercise`, which is
the `transition` policy, so **only the Game Master can**. That is why `ActionPause` and
`ActionTimeFactor` are declared and asked for by no caller — they were designed for this. Unlike
`order`, these two ARE policy-expressible: *"is this caller a Judge of this game"* is a role and a
domain, not an instance comparison. Recorded as a fix to make, not a deferral.

## B21 — Messaging ships a label-only `Derajat` and three classification values, both narrower than the concept

**Recorded 2026-09-22.** The client settled the two descriptive fields of the Telegram format — the
degree (`[7.7]`) and the classification that `[7.12]`'s border colour is specified in terms of — and
both decisions deliberately ship *less* than the concept leaves room for. Recorded here so the
narrowing is a decision on the record rather than an oversight, and so the smaller shape is not
later mistaken for the whole requirement. The answers themselves are in `docs/client-questions.md`
under `A3b` and `A11`.

### 1. `Derajat` is stored and displayed, and does nothing

The concept marks it undecided **twice** — `[7.7]` says only *"Manual Input or Dropdown (TBD) eg.
Segera"*, and `[7.12.8]` renders it as *"Degree - `?`"* with both children also `?` — so there is no
concept-derived behaviour to implement. It ships as a lookup value on the message: stored,
displayed, attached to nothing.

**The deferred reading is the interesting one.** TNI AL correspondence uses the degree as a
*precedence* — `BIASA / SEGERA / SANGAT SEGERA / KILAT` — each grade carrying a prescribed handling
time. Under that reading the functional receipt matrix the client already asked for (`A5`: the
system records when each recipient read the message) gains a threshold: a receipt is on time or it
is not, and Closure can report that a `KILAT` message went unread past its target. That is what
would make `[7.12.8.1]`'s *"Action"* and `[7.13]`'s *"Time Received"* into more than a log.

**It is deferred rather than dropped, and deferring costs nothing, because the concept states no
target for any grade.** Building it today would mean inventing the numbers and presenting them as
requirements. What matters is that the label-only shape does not foreclose it:

- the message already carries its grade, so nothing about `messages` changes;
- a target per grade is a column on the degree lookup table, which is additive;
- "overdue" is then a derived comparison of a receipt against that target, needing no new event;
- and the read receipts it would need are being built anyway for `A5`, so the expensive half is
  already being paid for.

To pick it up, the client supplies the time target per grade and says whether the system must
**display** an overdue state — the second half is a real question, because a target nobody can see
is indistinguishable from no target.

### 2. `SANGAT_RAHASIA` is not shipped, and is absent from the schema rather than unmapped

The classification field goes in with **three** values: `TERBUKA`, `TERBATAS` and `RAHASIA`. The
fourth the concept's question implies — `SANGAT_RAHASIA` — is held for a future update.

The distinction that matters is that it is **left out of the schema, not added with an empty
rendering rule**. `[7.12]` gives the border colour to the classification, and the client has
hard-coded exactly two: `RAHASIA` red, the rest black. A `SANGAT_RAHASIA` row in the database would
therefore be a value two clients must draw and have no colour for — the failure mode where a
backend ships ahead of a decided design and each frontend invents its own answer. Adding it later is
an additive migration (one constraint value plus one colour constant in the server — see §3), which
is cheaper than a rendering hole now.

**Consequence for the audience constraint.** `A1` gates `RAHASIA` on the recipient list and the same
reading covers `TERBATAS`, so only `TERBUKA` can be the unaddressed/broadcast case that `A2`
describes. The check constraint from `A2`'s knock-on ("at least one recipient **or** a broadcast")
therefore has the classification on one side of it, rather than being a rule about recipients alone.

### 3. The border colour is served, so no client implements the rule

**Decided 2026-09-22, after Flutter reported that it does not render the message border.** `[7.12]`
requires the colour, so it is a requirement rather than a styling choice and the only open question
is who computes it. It is the **server**: the classification is returned **with** the colour a client
should draw, and a client applies it without knowing why it is that colour.

The argument is not that presentation belongs in the API. It is that **the rule has one right answer
and two consumers**: with the mapping in each client, the Rust command centre and the Flutter handset
can disagree about the same message, and a Judge looking at a black border beside a Commando's red one
has no way to tell which is correct. Serving the colour also turns a rule nobody has implemented into
one that cannot be skipped — which is the concrete thing that went wrong here.

The shape settled on — **not in `minos-api.yaml` yet, because no message path exists**:

| field | type | value |
|---|---|---|
| `classification.code` | string, enum | `TERBUKA` \| `TERBATAS` \| `RAHASIA` |
| `classification.label` | string | display text for the code |
| `classification.border_color` | string | a `#RRGGBB` hex colour to draw directly |

Hex rather than a token such as `"red"`, because a token is a mapping the client would still have to
perform — which is the thing being removed. The field is named for **what it decorates** rather than
for the colour itself, so a client cannot read it as the message's text colour or as a map marker.

The two values in force are **`#000000`** for `TERBUKA` and `TERBATAS`, and **`#FF0000`** for
`RAHASIA` — the literal reading of *"Red for Classifed (Rahasia) and Black for the other"*. A deeper
red reads better as a border if the client wants one; it is a single constant either way, which is
the point of §3.

**Consequence for the fourth value, and it gets cheaper rather than dearer.** Because the server owns
the map, adding `SANGAT_RAHASIA` later is one constraint value plus one colour constant in one
process — not a colour map in each client to be kept in step. §2's argument for leaving it out is
unchanged; what changes is the size of the bill when it comes in.

**And it settles how the classification may be stored at all: not as a client-authored lookup.**
Every other vocabulary in this system is a lookup the operator populates — `Jenis` above, warning
codes (`B7`), unit statuses, unit types. The classification cannot be, because **its values are not
labels; they carry semantics**: an audience rule (`A1`, `A2`) and a colour. A lookup row authored in
the CMS would have neither, and would be exactly the rendering hole §2 describes — except one a user
could create without a deploy. It is therefore a fixed enum in the database with its semantics in the
server, and a fourth value is a migration rather than a row.

## B22 — Concealing an assumed sender is done in Go, not by row-level security

**Decided 2026-09-22.** Q21 asked for row-level security so that a message sent under a scenario role
does not expose its real author to the people who receive it. The client's answer is that **the Go
layer carries this rule and the database does not**, so `game_messages` carries no policy and the
concealment is a projection in `GameMessageService`.

**What that means concretely.** `RealAuthor` is ALWAYS read — the repository selects it on every read —
and the service decides whether it reaches a given caller. A recipient sees `sender.assumed_role` and
no author; the sender, the Game Master and the Judges see both. Realtime events never carry the author
at all, because an event is written once and has no caller to tailor it to; staff read it over HTTP
when they need it.

**Why RLS was the wrong instrument HERE — which is not the same as "RLS is bad".** This particular rule
is not a policy:

- The concealment is **per-caller, not per-row**. The same row is fully visible to a Judge and partly
  visible to a Commando, so a policy would have to know *who is asking*.
- Putting the caller into a policy means a **per-request session variable on every connection**, and a
  variable that is ever unset fails in one of two ways: it conceals the sender from the Judges too, or
  it conceals nothing. Neither is visible in a response, and both are worse than the deliberate rule.
- **A7 adds a second axis a policy cannot express.** An addressee who hid their copy stops seeing the
  message while the sender still sees *that they did* — so the same row's visibility differs by caller
  for two independent reasons at once.
- The "one definition" property RLS would have provided is already there: the visibility predicate is a
  single SQL constant (`messageVisibilityPredicate`) shared by the one-message read and the page read.

**What is given up, stated rather than glossed.** A query that bypasses the service — a future admin
tool, a `psql` session, an export job — sees the real author. That was already true of every other
concealed fact in this system, and it is the trade the client chose; the alternative was a rule that
cannot see who is asking.

