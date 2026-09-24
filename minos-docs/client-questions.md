# Open Questions for the Client

Running list. Status: `OPEN` / `ASKED` / `ANSWERED` / `DEFERRED`.

> **Round 2 (2026-09-14): most of sections A-E were answered — see `ROUND 2` at the
> bottom of this file, which supersedes the `Status` column below. The round-1 tables
> are retained as the record of what was asked.**
>
> **Rounds 3, 4 and 5 are appended below and are NOT reflected in the tables above.**
> They were raised later — writing the API contract (`G`), planning slice 2 (`H`), and
> adding taxonomy authoring (`I`) — so a row in sections A-E can be stale even though it
> says `OPEN`. Read the latest round before treating one as current. **Round 5 re-asks G2**,
> whose answer no longer matches the code.
Keep this file updated — several items here are contract-shaping and cannot be
reverse-engineered from the concept document.

---

## A. Blocking for the API contract

| ID | Question | Why it matters | Status |
|----|----------|----------------|--------|
| A1 | On **Classification**: is the gate "only the addressed recipients (TO/CC) may read a RAHASIA message", or does each *user* carry a clearance level? What do `TERBATAS` and `SANGAT_RAHASIA` change relative to `TERBUKA`? | Determines whether `user` needs a clearance attribute and whether RLS policies key off clearance or recipient-list membership | OPEN |
| A2 | Should a `TERBUKA` (unclassified) message be visible to **all** game participants, or only its TO/CC? | If TERBUKA = broadcast, the visibility model is a per-message flag; if not, it is always recipient-list based | OPEN |
| A3 | Exact allowed values for Telegram **`Jenis`** and **`Derajat`**. Is `Derajat` a 4-level precedence (`BIASA / SEGERA / SANGAT_SEGERA / KILAT`) or a free dropdown? | Currently modelled as enums; a free-text field changes validation + UI | OPEN |
| A4 | Confirm the meaning of **`Per`** in the signature block (`[7.14]`) and in the `PENGIRIM / Derajat / Waktu / Per / Paraf` table. Assumed: "via / on behalf of". | Field is currently a nullable free-text actor reference | OPEN |
| A5 | Is the **receipt matrix** (`Derajat.Aksi`, `Derajat.Tembusan`, `Waktu Terima`, `Waktu Tembusan`, `Paraf`) functional or display-only? I.e. must the system *record* when each recipient received/CC'd and who initialled? | Display-only = one nullable column set. Functional = per-recipient rows + write endpoints + realtime events | OPEN |
| A6 | For Administrative Messages, does **`Balas` / Reply** create a thread (reply-to chain), or is it just "send a new message back to the sender"? | Determines whether `message.reply_to_id` exists and whether Closure renders threads | OPEN |
| A7 | Confirm the **deletion asymmetry** is intended: sender deletes -> invisible to recipients but still visible to sender; recipient deletes -> still visible to sender *with a "deleted by recipient" tag*. Is that really wanted, or is the prototype behaviour (hide only for whoever deleted) the accepted one? | It is a gameplay/visibility decision, not a technical one. It affects what a Judge can prove during Closure | OPEN |
| A8 | Do **GM and Judges** always see deleted messages (including recipient-deleted ones) in Closure? | Audit/playback integrity. Assumed yes | OPEN |
| A9 | Confirm **`Waktu Pengunjukan`** `[7.9]` vs **`Waktu Pembikinan`** `[7.12.6]`: which one is the authoritative "message time" used for ordering and playback? | Playback ordering depends on it | OPEN |
| A10 | Confirm the **assumed-time timezone**. Can a scenario clock sit in a different timezone than the real event (e.g. assumed UTC while actual is WIB)? | `game.timezone` vs `game.assumed_timezone` | OPEN |

## B. Gameplay / simulation rules

| ID | Question | Why it matters | Status |
|----|----------|----------------|--------|
| B1 | When a **hierarchy node commander** (e.g. Satuan Tugas Alpha) issues an instruction, is it **advisory** (a Telegram message the unit Commando must voluntarily obey) or **binding** (the system directly changes the subordinate unit's heading/speed)? | Concept `[1.8]` says only the Unit Leader may "steer" the Unit, which implies advisory. Confirm — it is a core mechanic and it affects Judgement | OPEN |
| B2 | Can one person command **two or more hierarchy nodes** at once? Can a node have **two or more** commanders? | Uniqueness constraints on `game_node_commander` | OPEN |
| B3 | The prototype's speed slider runs **0–100 kn**, but the fleet's max is ~30 kn (submarines 11 kn surfaced). Should the system **reject or clamp** orders above the unit's max speed, and should the slider max be per-unit? | Accuracy. A Commando ordering 100 kn from a PC-40 is not judgeable | OPEN |
| B4 | With **turn-rate mechanics** enabled, what happens if a Commando orders a heading change larger than one tick allows — does the unit turn at max rate toward the target heading (standard), or reject the order? | Affects order semantics and Judgement | OPEN |
| B5 | For **`Mode = Static`** games, is movement disabled entirely, or just discouraged? | Mode is in Planning `[1.11]` but never defined in the concept | OPEN |
| B6 | What is the required behaviour when a **Commando disconnects mid-Execution** — does their unit keep moving on the last order indefinitely? | Assumed yes; needs confirmation as it affects "Not Responding" warnings | OPEN |
| B7 | Should the **Warning** list of codes be fixed (`OUT_OF_AREA`, `NOT_RESPONDING`, ...) or free? The concept only gives two examples | Enum design | OPEN |
| B8 | Does a **Judgement** need to be visible to the judged personnel, or is it Judges/GM-only until Closure? | Determines whether judgements are on the realtime channel at all | OPEN |
| B9 | What is the **score scale** (e.g. 1–5, 0–100)? Or purely free text for this release? | Currently `score numeric NULL` + `game.score_scale` | OPEN |

## C. Data / content

| ID | Question | Why it matters | Status |
|----|----------|----------------|--------|
| C1 | **Submarines**: the sheet gives max speed surfaced only (11 kn). Who supplies the **submerged** max speed (and snorkel speed) for Cakra and Nagapasa classes? | Blocking for a usable submarine movement model | OPEN |
| C2 | Review `docs/unit-classification-mapping.csv` (41 classes). Confirm the proposed `category_code` + NATO `type_nato` values, especially `Korvet / OPV` -> `PATROL_CRAFT` and the FF/FFG calls | This is the seed for unit master data; cheaper to fix now than after import | OPEN |
| C3 | Who is the **SME** that can correct derived values (turn rate, weapons fit, sensor fit)? Or should everything derived stay reviewable-but-unverified? | Derived turn rate is an estimate from LOA | OPEN |
| C4 | Future **Angkatan Darat / Angkatan Udara** integration: confirm this is genuinely planned, so the schema is shaped as class-table inheritance (`unit` + `unit_spec_vessel` / `unit_spec_aircraft` / ...) from day one rather than retrofitted | Retrofitting a single-table naval spec into multi-service is a costly migration | OPEN |
| C5 | For the future **Fire** feature: will weapon mounts need per-mount attributes (arcs, reload, magazine depth) or is a per-unit weapon list enough? | Do NOT build the weapon schema now; this only decides how the JSONB blob is shaped so migration stays mechanical | DEFERRED |

## D. Infrastructure / deployment

| ID | Question | Why it matters | Status |
|----|----------|----------------|--------|
| D1 | Exact **target OS + release** on the mini-PCs (Ubuntu 24.04? Debian 12?) and confirm amd64 | The offline Docker bundle must be built for the matching distro/libc | OPEN |
| D2 | Does the air-gapped network have **any time source** (NTP server, GPS clock, PTP)? If not, who designates one host as the time authority? | The product's accuracy is time accuracy. Unsynced hosts mean unreconcilable logs | OPEN |
| D3 | Are all services on **one host**, or will the DB/Redis/Garage be split across hosts? | Single host = no clock skew problem between services. Multi-host = NTP becomes mandatory | OPEN |
| D4 | Do the mini-PCs have a **GPU** / sufficient RAM for MapLibre GL rendering? | Affects whether the frontends should ship a raster fallback | OPEN |
| D5 | Is **TLS** required on the isolated LAN, or is plain HTTP + token auth acceptable on a trusted network? | Offline cert management is real work (internal CA, cert distribution) | OPEN |
| D6 | Who **generates and owns the map tiles**? OSM is ODbL — attribution is mandatory and derived tile sets are themselves ODbL. Does the client accept that licence? | Legal/attribution obligation shown on every map view | OPEN |
| D7 | Approve pinning `postgis/postgis:16-3.5-alpine` (PostgreSQL 16 + PostGIS 3.5) rather than PostgreSQL 18 | Image availability, maturity, and a smaller offline bundle | OPEN |
| D8 | Any requirement for **data retention / purge** after a game (e.g. Rahasia content must be destroyed after N days)? | Affects soft-delete (which we are doing anyway) but could demand hard purge + audit | OPEN |

## E. Scope / acceptance

| ID | Question | Why it matters | Status |
|----|----------|----------------|--------|
| E1 | Is **Closure playback** (the slider) in the 27–28 Sep acceptance criteria, or can it ship as "timeline + map state at time T"? | It is the single most expensive remaining feature | OPEN |
| E2 | Is **Telegram form fidelity** (all 15 header fields + per-recipient receipt table) part of the demo, or is a subset acceptable on the 27th? | Large surface across two frontends | OPEN |
| E3 | What exactly is being **demonstrated** on 27/28 Sep — a scripted scenario, or free client play? | Changes how much robustness/edge-case handling is required | OPEN |
| E4 | Are **PDF/Excel export** and **playback export** expected, and by when? | Deferred, but needs a date | OPEN |
| E5 | Do the Flutter and Rust teams have their **API contract consumers** ready? Who is the integration contact for each? | Contract-first only works if both consumers can start immediately | OPEN |

---

## Answered (for the record)

| ID | Decision |
|----|----------|
| Q1 | `Skala Waktu Dasar` is the **base** value, and it is what the GM edits throughout the game |
| Q2 | Geographic map, OSM-derived, must work **offline** |
| Q3 | Turn-rate mechanics **in scope**, but ship `instant` mode first for the demo |
| Q4 | All units visible to all once Execution starts; distance/bearing is an **on-demand** query |
| Q5 | **No combat simulation** this release; engines to be added as specs later |
| Q6 | Judgements are **free text** with **per-action** attachment capability |
| Q7 | Breach/warnings are **100% manual**, no geofence |
| Q8 | Random events are **manual**; no natural disasters (`COMM_BLACKOUT` is the example) |
| Q9 | Pause: assumed clock stops, actual clock keeps running, actions **rejected not queued** |
| Q10 | Hierarchy: **master tree** in CMS, snapshotted into the game and configured per game |
| Q11 | `[5]`-style brackets are **footnote references**, not limits. GM may also be a Judge |
| Q12 | Commando is **1:1 with a Unit**; a person may additionally command one hierarchy node |
| Q13 | Impersonation: recipient sees **assumed identity only**; DB stores the real sender |
| Q14 | **English canonical** for codes/DB/API |
| Q15 | Backend only. VPS for testing, **air-gapped** for delivery. Ubuntu/Debian, amd64 |
| Q16 | Scale: **~100 units** per game, ~10 concurrent games |
| Q17 | Stack: Go, PostGIS, Redis, Centrifugo, Garage — all Dockerised |
| Q18 | Personnel **rejoin mid-state** |
| Q19 | PostGIS/PostgreSQL version to be confirmed from Docker Hub availability (see D7) |
| Q20 | Asset data: auto-increment PK, hull number as natural unique key, `—` -> NULL, Indonesian number locale normalised on import |
| Q21 | RLS is acceptable (and wanted) for hiding sender identity from personnel |
| Q22 | Classification **is an access-control boundary**, scoped to users assigned to the game |
| Q23 | Hull classification: keep the source `Kategori`/`Kelas` for traceability **and** add NATO symbols |

---

# ROUND 2 — answers received 2026-09-14

This section supersedes the `Status` column in the tables above.

## Closed in round 2

| ID | Answer |
|----|--------|
| A1 | **No clearance levels.** `RAHASIA` is readable only by the addressed recipients (TO + CC). So the gate is recipient-list membership, not a user attribute |
| A2 | If TO **and** CC are both omitted, the message is visible to **all** game participants (broadcast) |
| A4 | Assume "via / on behalf of". Nullable free-text actor reference |
| A5 | The receipt matrix records when each recipient **read** the message. So it is a functional read receipt, not display-only |
| A6 | Reply creates a **thread** |
| A7 | **Confirmed** asymmetric deletion (see knock-on effects below) |
| A8 | Yes — GM and Judges always see deleted messages, including recipient-deleted ones |
| A9 | **`Waktu Pembikinan` (create time)** is the authoritative message timestamp |
| A10 | Frontend supplies its timezone; backend returns RFC3339 adjusted to it; DB stores **UTC**. Assumed time may sit in a different timezone from actual time |
| B1 | **Always advisory.** Only a unit's own Commando steers it. A node commander who is also a unit Commando performs two separate acts: send the Telegram message, then manually adjust his own unit |
| B2 | 1 node == 1 commander. 1 person == at most 1 node. A person may hold both one node and one unit |
| B3 | The prototype's 100 kn slider is a **bug**. Clamp to the unit's spec maximum; slider max is per unit |
| B4 | Turn at max rate toward the ordered heading |
| B5 | `STATIC` = no simulated movement; a drag-and-drop board. Readiness gate is more lenient, and replay is supported (expected to look janky — intended) |
| B6 | The unit continues on its **last order** until a new order (for example a stop command) arrives |
| B7 | Warning codes as a **lookup table** the client can populate |
| B8 | **Judgements are GM/Judges only** — never shown to unit or node Commandos |
| B9 | Free text input for this release |
| C1 | The **CMS Operator** is responsible for providing accurate values, including submarine submerged speeds |
| C3 | Same — the Operator reviews and updates unit specs as they evolve over time |
| C4 | AD/AU integration is **confirmed planned** -> class-table inheritance from day one |
| C5 | Fire simulation needs per-mount fidelity "with certain limitations" -> keep the JSONB shape mount-capable, but build no weapon schema yet |
| D1 | **Debian 13 (trixie)**, amd64 confirmed |
| D3 | **Single host** for both the dev VPS and the demo Mini-PC |
| D5 | Dev: Let's Encrypt. Offline: **self-signed is acceptable** |
| D6 | Client accepts the ODbL licence; OSM attribution is shown in the demo |
| D7 | **Approved:** `postgis/postgis:16-3.5-alpine` |
| D8 | Export deferred to a separate product; does not affect core functionality |
| E1 | **Playback IS in the acceptance criteria** |
| E2 | A **subset** of Telegram fidelity is acceptable |
| E3 | Demo covers **both** the game flow (Planning -> Preparation -> Execution -> Closure) and the CMS side |
| E4 | Export is **not** in this deadline's acceptance criteria |
| E5 | Frontend teams are actively working (no named integration contact yet) |

## Knock-on effects to design around

1. **B8 is settled — follow the client's answer.** Judgements are GM/Judges only and
   never reach unit or node Commandos. An earlier assumption of ours that judged
   personnel might see their own scores is **superseded**. Two consequences:
   - Judgements never touch the Centrifugo game channel at all — they are a
     Closure-side concern only, which removes a realtime fanout path entirely.
   - Concept `[1.6]` says Judges "give Points to a **Personnel**". With export deferred
     by `E4`, nothing in the system surfaces scores to personnel. Recorded as `F6` —
     a requirement question for the client, not a design contradiction.
2. **A2 makes classification functional.** Because an unaddressed message broadcasts, we
   need an explicit audience concept: either named `TO`/`CC` recipients, or `ALL`. Add a
   check constraint: a message must have at least one recipient **or** be flagged as a
   broadcast.
3. **A9 fixes the playback ordering key** to `created_at` (Waktu Pembikinan), stored as
   `timestamptz`, with `assumed_at` used for timeline placement.
4. **A7 deletion asymmetry, precisely:**

   | Actor | Effect |
   |-------|--------|
   | Sender deletes | Hidden from **all** recipients. Still visible to the sender, GM and Judges, marked deleted |
   | Recipient deletes | Hidden from **that recipient only**. The sender still sees it, with a "deleted by recipient" badge |
   | Anyone | **Never hard-deleted.** GM and Judges always retain the record |

   Playback must render the **visibility as it was at that instant**, with delete events
   placed on the assumed timeline. Otherwise a Judge reviewing the game sees a message
   that the personnel provably could not have seen.
5. **B5 `STATIC` mode needs its own acceptance path** — no movement, drag-and-drop,
   lenient readiness, replay. That is a second demo mode, not a config flag.
6. **D3 single host simplifies the clock problem** considerably: all containers share
   the kernel clock, so only the host's own drift matters. Machine-to-machine skew is
   not a concern. `D2` (does a time source exist?) is therefore lower risk but still
   open.

## New questions raised in round 2

| ID | Question | Why it matters | Status |
|----|----------|----------------|--------|
| F1 | The v2 sheet's `Satuan / Koarmada` column reveals the **peacetime administrative organisation** (Satkor/Satkat/Satfib/Satban/Satpat/Satran/Satsel x Koarmada I/II/III, Lantamal I-XIII, Kolinlamil, Pushidrosal, AAL). This is a **different tree** from the concept's task organisation (Unsur / Satuan Tugas / Gugus / Operasi Gabungan). Should the CMS carry **two** hierarchies — administrative (for the roster and master data) and task organisation (per game)? | Changes the CMS master-data model, and whether `unit` keeps a permanent org path in addition to its game-time assignment. The concept only describes the task tree | **ANSWERED — one tree, the game hierarchy** (see the answer below) |
| F2 | Some `Catatan` values are prefixed `*` (for example `*Ex-Volksmarine 1993; ...`, `*Dibangun 1960-an...`). Does the asterisk mean "unverified / estimated"? Should unverified values be flagged in the data model? | If yes we need an `is_verified` / `data_confidence` flag on the spec version, rather than losing that meaning on import | OPEN |
| F3 | v2 contains a **duplicate row**: `KRI Lumba-Lumba` (hull `858`, Pari/PC-40, Lantamal IX) appears at rows 68 and 69, differing only in commission year (`2013-2020` vs `2019`). One must be removed or corrected — which? Separately confirm that `KRI Dewaruci` and `KRI Bima Suci` legitimately have **no hull number** | Import behaviour and key design. Also affects the "Pari 11" count in the rekap sheet | OPEN |
| F4 | **ANSWERED 2026-09-14** — ignore all location data. `Latitude`, `Longitude`, `Link Google Maps` and `Keterangan Posisi` are **not imported at all**. They were unusable anyway (17 unique coordinates across 125 ships) | Removed an import hazard and a data-quality argument in one decision | ANSWERED |
| F5 | During Execution, do GM/Judges need to see **who** an assumed identity belongs to, or is unmasking only needed at Closure? | Scope of the RLS policy and the impersonation API surface | OPEN |
| F6 | With export deferred (`E4`) and judgements hidden from personnel (`B8`), **how are scores delivered to personnel?** Nothing in the app surfaces them | Possible unstated requirement, and it affects whether a deliberately de-scoped export becomes a hard dependency | OPEN |
| F7 | The CMS will be edited by an Operator who is responsible for accuracy (`C1`, `C3`). Do spec edits need an **audit trail** (who changed what, when, from what value)? | We are versioning specs as immutable rows anyway, which makes this nearly free — but it needs to be a stated requirement rather than an accident | OPEN |

### Answer received 2026-09-15

**F1 — one hierarchy, and it is the game's.** The v2 workbook's peacetime administrative
organisation (`Satuan / Koarmada`, Lantamal, Kolinlamil, Pushidrosal, AAL) is **out of scope**:
the v2 sheets are ignored for this purpose. The only tree is the task organisation the concept
describes (`Unsur` / `Satuan Tugas` / `Gugus` / `Operasi Gabungan`), and it belongs to a
**game** — a node exists because a game created it, not because the peacetime navy has one.

Consequences, all of which simplify slice 2:

- **No administrative-hierarchy master data.** No peacetime org tree in the CMS, and no
  permanent organisational path on `unit` alongside its game-time assignment.
- **Hierarchy is game-scoped**, so its tables carry `id_game` and are built per game. A hull's
  position in a tree is a fact about a *game*, never about the hull. This is the same reasoning
  as the append-only specification versions: one hull may sit under different nodes in two
  different games, and neither assignment is its real parent — so there is nothing to keep in
  sync when a game is deleted.
- It also means the roster and the organisation cannot disagree. With a second, permanent tree
  there would be two answers to "what is this hull part of", and a game would have to reconcile
  them.

`C2` (the classification mapping) remains the only outstanding master-data question about the
roster itself.

## Still open from round 1

- **A3** — `Jenis` / `Derajat` allowed values. **Split on 2026-09-22** after re-reading `[7]`,
  because these turn out to be two different kinds of question and only one of them needed an
  answer before the messaging build could start:

  - **A3a — `Jenis` (message type).** A lookup table the client populates. No behaviour attaches
    to it in the concept, so the *values* are still wanted but nothing is blocked by them.
  - **A3b — `Derajat` (degree / urgency). DECIDED 2026-09-22 — a label, for now.** The concept
    mentions it twice and **declines to define it both times**: `[7.7]` says only *"Manual Input
    or Dropdown (TBD) eg. Segera"*, and `[7.12.8]` renders it as *"Degree - `?`"* with both of
    its children also `?`. So there is no concept-derived behaviour to build, and the field ships
    as **stored and displayed, with nothing attached to it** — a lookup row on the message, exactly
    as `Jenis` is. That settles `[7.7]`'s own *"Manual Input or Dropdown (TBD)"* in favour of a
    dropdown, while leaving a later switch to free text a validation change rather than a schema
    change. The second reading below is real and is **deferred rather than dropped** — recorded as
    `B21` in `docs/backlog.md`:
    1. **A label.** *(chosen)* Stored and displayed. Purely additive.
    2. **A precedence carrying a handling time.** TNI AL correspondence uses
       `BIASA / SEGERA / SANGAT SEGERA / KILAT` with a prescribed time target per grade, which
       would make the receipt matrix (A5) mean *"was it read within this grade's target"* and let
       Closure report messages that went overdue. The concept states no target for any grade, so
       this cannot be built without the client's numbers — but it needs nothing from the client to
       *defer*, which is why the label shape ships now.

  Two things `Derajat` is **not** — stated because an earlier draft of our own notes got the first
  one wrong and it would have put the field in the wrong place:
  - It does **not** drive the content border colour. `[7.12]` gives that to the *Classification*
    (*"Red for Classifed (Rahasia) and Black for the other"*).
  - It is **not** the gate on who may read a message. That is Classification, answered by A1.

- **A11 — the field list in `[7]` has no Classification field, yet `[7.12]` depends on it.
  Raised 2026-09-22, DECIDED the same day.** The 15 items of `[7]` give no place to record the
  classification, but `[7.12]`'s border colour is specified in terms of it and both A1 and A2 were
  answered about it — so the field is *required by the answers and missing from the list*. It is
  added as a per-**message** classification.

  **THREE values are in force, and the fourth is held back deliberately:**

  | value | border colour | who can read it |
  |---|---|---|
  | `TERBUKA` | black | every game participant, when TO and CC are both omitted (A2) |
  | `TERBATAS` | black | the addressed recipients only |
  | `RAHASIA` | **red** | the addressed recipients only (A1) |
  | `SANGAT_RAHASIA` | — | **not shipped.** Held for a future update |

  The colours are **hard-coded, not configurable** — the client asked for exactly that, so there is
  no colour column and no lookup table. **And the server sends the colour rather than the rule**
  (decided 2026-09-22): the classification is returned together with the border colour a client
  should draw, so neither frontend implements the mapping. Flutter reported it does not render the
  border, and a rule a client must implement is a rule that does not get implemented; with the map
  on the server, the Rust command centre and the Flutter handset cannot disagree about the same
  message. Shape and reasoning are in `B21` of `docs/backlog.md`.

  That also settles **how the classification may be stored**: not as a client-authored lookup table
  like `Jenis`, because its values are not labels — each one carries an audience rule (`A1`, `A2`)
  and a colour. A lookup row authored in the CMS would have neither. It is a fixed enum with its
  semantics on the server, and a fourth value is a migration rather than a row.

  Two consequences, both following from answers already given rather than from this one:
  - **`SANGAT_RAHASIA` is absent from the schema, not merely unmapped.** Adding it later is an
    additive migration — one constraint value plus one colour constant, both in the server — whereas
    shipping it now would create a value the border colour has no rule for, which is a rendering
    hole in two clients. See `B21` in `docs/backlog.md`.
  - **Only `TERBUKA` may be unaddressed.** A1 gates `RAHASIA` on the recipient list and the same
    reading covers `TERBATAS`, so the broadcast case that A2 describes is reachable only from
    `TERBUKA`. The check constraint in A2's knock-on ("at least one recipient **or** a broadcast")
    therefore has the classification on one side of it, rather than being a rule about recipients
    alone.

  Note this is a different "classification" from the per-**unit** taxonomy (`Q22`, `C2`) — the two
  happen to share a word.

- **D2** — is there any time source on the air-gapped network? (no answer yet)
- **D4** — Mini-PC GPU capability (client gave no answer; **we assume no discrete GPU**, integrated graphics only)
- **C2** — client review of `docs/unit-classification-mapping.csv`
- **C5** — weapon/fire fidelity (deferred; decide the JSONB shape only)

# ROUND 3 — raised while writing the API contract (2026-09-14)

These came out of freezing OpenAPI slice 1 (auth + master data). Both are
authorization questions, and both are cheap to answer now and expensive later:
each one changes the shape of an endpoint rather than its internals.

| ID | Question | Why it matters | Status |
|----|----------|----------------|--------|
| G1 | When an Operator edits a user account, may they edit **any** account, or only accounts within **their own unit or hierarchy node**? | This is a row-level rule. Casbin can answer "may this role update `/system/users`" but it cannot answer "may this *Operator* update *this* account", because policy objects deliberately carry no instance ids. If the answer is "own unit only", every administration endpoint needs an explicit ownership check in the service (`403`), backed by row-level security in the database. If the answer is "anyone", the endpoints stay simple and the check is unnecessary. **Note that RLS alone is not a sufficient answer**: it would report a blocked edit as `404`, indistinguishable from a missing account, which is poor evidence in an audit trail | **ANSWERED — any account** (see below) |
| G2 | Can a lookup row (a unit status, a category, a NATO type, a service branch) ever be **retired or deleted**, or only renamed and added to? | Lookup tables currently have no `deleted_at` column at all, so a row can only be edited — it can never be removed or hidden. Every *other* table in the schema is soft-deletable. If the CMS needs to retire a category (say `Korvet / OPV` after the workbook's categories are normalised), that column has to exist **before** the rows are seeded, because a partial uniqueness index cannot be created on a column that is not there. Answering "only rename" is also a fine answer, and cheaper | **ANSWERED — rename only** (see below) |

### Answers received 2026-09-14

**G1 — an Operator may administer any account.** This is not connected to the game.
Inside the game, a Commando cannot take administrative action such as editing their own
unit's accounts. So the rule is about *which application you are in*, not about which unit
rows you can see: administration is an out-of-game function, and the game's authorization is
a separate concern that does not overlap with it.

Consequences, all of which simplify the code:

- No per-row ownership check in the user administration endpoints. The Casbin check on the
  resource family is the whole rule, so no service method needs to compare an actor's unit
  against a target's.
- `GET /users` does not filter by unit. This removes the inconsistency flagged in the note
  below — the list shows every account to anyone allowed to call it, which matches the edit
  rule exactly.
- No new database work. `users` already carries the audit columns, so an administrative edit
  is attributable without any additional table.
- **Out-of-game administration must not be reachable with an in-game identity.** Since the two
  concerns are separate, the `Commando` role must not hold any `Administrator` policy. That is
  a Casbin policy question, and it is worth a test: a Commando token calling
  `PATCH /users/{id}` must be refused. If the roles are ever granted in a way that overlaps,
  this is the assertion that catches it.

**G2 — lookup rows can only be renamed, never retired.** No `deleted_at` column is added to
any lookup table, and none is needed. The uniqueness guarantees stay as they are.

Consequences:

- `migration 000006`'s partial index on `unit_classes(name)` is unaffected — a lookup row is
always live, so a plain unique index would also have done. Nothing to change, and importantly
nothing to re-run.
- Lookup tables keep exactly one delete verb: none. `HelperTables` is read-only through the
  API, and the absence of `deleted_at` is now a deliberate design decision rather than an
  omission — which is worth a line in the schema comments so a later reader does not "fix" it
  by adding the column.
- Renaming a lookup row is safe because dependent rows reference it by id, not by name. The
  workbook's normalisation (`Korvet / OPV` and friends) is therefore a pure `UPDATE`, and any
  client that cached the old label refreshes on the next read.
- One consequence worth stating: a lookup value that has been used in a recorded game can be
  renamed after the fact, which retroactively changes how that history reads. For the demo
  this is fine; if the client later wants historical labels to be immutable, the answer is a
  snapshot on the game record, not a `deleted_at` on the lookup table.

### Notes on the two, for whoever answers

**G1** has a second-order effect worth stating: it also decides whether
`GET /users` should filter by unit. A list endpoint that shows every account to an
Operator who may only *edit* their own unit's accounts is inconsistent in a way
participants will notice.

**G2** is not urgent for the demo, but the decision is cheapest to take right now —
while the lookup tables are only seeded and nothing depends on their shape.

# ROUND 4 — raised planning slice 2 (2026-09-15)

These came out of designing what was then called the **Operational** game mode, where a mobile app
on a real vessel reports its position. The design is recorded in `docs/game-modes.md`; these are the
points where a client answer changes the *contract* rather than our internals.

> **Operational is no longer a game mode** — it is a **live feed**: real vessels, no game, no
> participants, no state machine, no orders. Read "in Operational mode" below as "in the live feed".
> Every question still applies except **H10**, which is moot, and **H12**, which is answered for the
> feed (the beacon displays nothing; the command centre sees everything). See
> `000014_narrow_game_mode`, and the note at the top of `docs/game-modes.md`.

| ID | Question | Why it matters | Status |
|----|----------|----------------|--------|
| H1 | In Operational mode, is the person operating the reporting device the **unit's own Commando**, or separate personnel (a signaller, a deck officer)? | It decides whether the binding is *user → unit* or *device credential → unit*, which are different tables and different checks. Maneuver binds a Commando to a unit; if Operational binds a device to a vessel, the two modes no longer share an assignment model | **ANSWERED — the unit's own Commando** (see below) |
| H2 | Confirm the liveness rules: reports every **5 s**, drawn as live until **15 s**, treated as lost after **60 s**, all configurable per game. | These set the map's behaviour and the point at which the GM may take over. They are configuration, so a wrong number is cheap — but the *shape* (three states rather than one) is a contract decision | OPEN |
| H3 | Confirm the GM may move a unit **only** when it is *lost* (>60 s without a report) — and that a live websocket connection is **not** by itself enough to block a manual move. | A device that is connected but silent (app backgrounded, no GPS lock) needs a manual correction. Gating on connection state rather than report recency would block exactly that case, and would let the GM move a vessel whose reports are still arriving | OPEN |
| H4 | If a device is swapped mid-exercise, must the swap be recorded as an **audited event** (who, when), and must each position report carry the device that produced it? | A swap usually brings a small discontinuity — different receiver, antenna, calibration — that reads as the vessel jumping. Without the device stamped on each report nobody can explain it afterwards, and a gap follows every swap | OPEN |
| H5 | For reports buffered while offline: accept **all** of them flagged as backfilled, or refuse anything older than a limit? | Accepting with a flag keeps the evidence. Refusing loses real data, but means a device cannot submit a fabricated track for a long-dead window after the fact. Our recommendation is accept-and-flag | OPEN |
| H6 | Does the reporting device supply **course and speed** (most marine GPS units report SOG/COG), or must the server derive them from consecutive positions? | Deriving is strictly worse — noisy, and dependent on report cadence, so a slow reporter looks like a vessel that cannot hold a heading. Anything we derive will be marked as derived rather than measured | ANSWERED |
| H7 | Confirm the **Mini-PC is the designated time authority** for the exercise (this closes D2). It will serve NTP on the isolated LAN; note mobile devices cannot be pointed at it for system time, so the server timestamps reports instead. | Fixes what "the correct time" means during an exercise: one consistent clock, not necessarily an absolutely correct one | OPEN |
| H8 | Is deferring **Static** mode acceptable for this release, with Maneuver shipping first? | B5 noted Static needs its own acceptance path (drag-and-drop, lenient readiness, janky replay). If it is in the acceptance criteria we need to schedule it rather than discover it late | OPEN |
| H9 | When a unit goes **lost** during Execution, does the system raise a **warning** automatically, or is that the GM's judgement to record? | **ANSWERED by the concept itself** — see below. Warnings are given by the GM and Judges when *they* determine a breach, so the system's job is to surface the condition, not to accuse | ANSWERED |

### Answer received 2026-09-15

**H1 — the device is operated by the unit's own Commando.** So the binding is **user → unit**, the
same assignment Maneuver uses, and the two modes share **one assignment model**. The mode decides
what an assignment *does*, not what it *is*.

Three consequences, all simplifying:

- **No separate device credential.** The app is signed in as the Commando, so it authenticates
  with the ordinary session and needs no second identity to issue or manage. The device
  identifier is still recorded (H4) — to audit a swap and explain a discontinuity in the track —
  but it grants nothing.
- **The assignment tables are shared between modes**, so steps 4 and 5 of the slice-2 plan are no
  longer blocked.
- **The who-may-report rule becomes an instance check, not a policy.** "May this user report for
  *this* unit in *this* game" cannot be a Casbin object, because objects name resource families
  and carry no instance dimension — the same reason `/users/me` carries no permission check. So it
  is a service-level comparison against the assignment, with Casbin answering only the coarse
  question (may this participant make per-object edits in this game's domain).

A failed check is **403, not 404** — the G1 reasoning: reporting a blocked action as "not found"
is indistinguishable from a genuinely absent row and leaves a hole in the audit trail.

## Note on H9 — answered by the concept, not by the client

Concept Execution `[3.8]`: the GM and Judges *"are allowed to give **Warning** to each Personnels if
they are determined to 'Breach' certain aspect (Moved to far from the Game Area, Not Responding,
etc)"*. So a warning is a **human act of judgement**, recorded against a personnel.

That settles the question in the useful direction: the server must **not** emit warnings by
itself, because a warning is an accusation and the concept places it with the people running the
exercise. What the server owes them is the **condition** — and one of the concept's own examples
is *"Not Responding"*, which in the live feed is exactly the lost-vessel state from H2.

So the split is:

| Who | Does what |
|-----|-----------|
| Server | deterministically detects and shows "this unit has not reported for 62 s" |
| GM / Judges | decides whether that is a breach, and records the Warning if so |

That also means "lost" does not need to be an auditable event in its own right — it is a derived
display state. The auditable event is the Warning, if a human decides to give one. This removes the
tuning problem noted below: an auto-warning would need its own threshold and would fire on every
coverage gap.

# ROUND 5 — raised adding taxonomy authoring (2026-09-17)

These come from making `unit_categories` and `unit_types` **operator-authorable** for the first time
(migration `000024_taxonomy_authoring`, with `POST`/`PATCH`/`DELETE` on both and
`PUT /service-branches/{id}/unit-categories`). Until now nothing could write any lookup row — see
B5 — so several answers below were given about a surface that did not exist.

**I1 is not a new question. It is G2 asked again, because the answer no longer matches the code.**

| ID | Question | Why it matters | Status |
|----|----------|----------------|--------|
| I1 | **G2 answered "lookup rows can only be renamed, never retired — lookup tables keep exactly one delete verb: none."** `DELETE` routes now exist for `/unit-categories/{id}` and `/unit-types/{id}`. Which stands? | This is the one question here that changes behaviour rather than documentation. The exposure is specific: `is_system` is a client-side signal only, the foreign key protects a type only when a class points at it, and **18 of the 19 seeded unit types have no class** — so 18 of them are deletable by anyone holding the `delete` action, with no tombstone and no undo. Note that `000019` (a day after G2) describes the first operator-authored lookup as "hard-deleted, never soft-deleted", so the project's own record disagrees with itself. Options: keep DELETE as built; drop DELETE and keep `POST`/`PATCH`; or keep DELETE but refuse `is_system` rows server-side so only operator-created rows can go | **RESOLVED 2026-09-21** — see the note below |
| I2 | Which categories should **Army** and **Air Force** own? They currently own none, so no hull for either can be created. | With the ownership rule in force, a hull's branch must own its class's category — and the ten seeded categories all belong to Navy. Army and Air Force therefore cannot field anything, and `GET /unit-categories?id_service_branch=2` returns `[]`. This is the mapping only the client can supply; the routes to enter it now exist | OPEN |
| I3 | Should the lookup lists be ordered by the label the operator **reads**, rather than by `name`? | Every lookup is `ORDER BY name ASC`, and for categories `name` is the English label while the screen draws `id_name` — so the Indonesian list is not in Indonesian alphabetical order. Separately, the comparison is case-sensitive on the current image (Alpine/musl), so a category typed `survey` sorts after every capitalised one and reads as having vanished. Changing it is one line in the shared lookup read, but that read backs **all ten lookups** and every payload embedding one, so other screens reorder too. Leaving it is a fine answer | OPEN |
| I4 | Do operators need a **`note`** on a lookup row that they can read back? | `HelperPublic` deliberately omits `note`, so the authoring routes accept no note: a field a client can set but cannot read is worse than one it cannot set. If notes are wanted, `note` has to be exposed on the shared lookup shape, which affects all ten lookups and both generated clients | OPEN |

## Note on I1, for whoever answers

G2 was answered when the taxonomy was seeded master data with no write path, and its reasoning was
about **soft** deletion — "no `deleted_at` column is added to any lookup table, and none is needed".
The routes as built do a **hard** delete, so no `deleted_at` was added and that half of G2 holds
exactly. What is new is that deletion is now *expressible* at all, and that the rows G2 was most
concerned with — the NATO types the fleet's vocabulary is built from — are the ones with no foreign
key standing behind them.

The cheapest answer that satisfies both records is the third option: keep `DELETE`, refuse
`is_system`, and an operator can undo their own mistake without being able to delete the seed.


## I1 RESOLVED 2026-09-21 — G2 is deliberately reversed, and the DELETE is gone

The client settled this, and the answer was neither of the two options this note recommended. **The
DELETE was removed entirely and retirement replaced it**, cascading from the row named to everything
beneath it.

**Why a tombstone, when G2 said none was needed.** G2's reasoning was about SOFT deletion and it was
not wrong — what changed is the verb. A hard delete cannot express a cascade: every foreign key in this
hierarchy is `ON DELETE NO ACTION`, so a category with types under it could not be deleted at all, and a
hull that has ever been in an exercise could not be deleted either. Migration `000029` adds
`deleted_at`/`deleted_by` to `unit_categories` and `unit_types`, which brings all four levels —
category, type, class and hull — to the same shape. Two of them already had it.

**What the client asked for, in its own words.** The operator interacts with all four levels, "Retire
results in all object under the retired parent to be deleted as well and this is intended, the frontend
will handle the confirmation themselves."

**So the third option was considered and declined, and the reason is for the frontend teams.**
`is_system` does NOT block a retirement. All ten categories and all nineteen types are seeded, so
refusing system-owned rows would have made the feature do nothing whatsoever. **`HelperPublic.IsSystem`
must therefore no longer be used to withhold the affordance** — the contract said to disable the delete
on those rows, and that instruction has been withdrawn in `minos-api.yaml` wherever it appeared. A
client that keeps it will find `POST /unit-categories/{id}/retirement` unreachable for every category
the fleet actually has.

**What replaces the safety the hard delete lacked.** Three things, none of them a refusal, because a
refusal was the problem: the retirement returns the COUNTS it withdrew, so the effect is reported
rather than inferred; the confirmation lives in the client, which is where the client asked it to live;
and a subtree deployed in a running exercise is refused with `409` plus `data.units_in_execution` — a
hull vanishing from a live order of battle is the one outcome that is not a master-data edit. Planning
and closure are not refused.

**`000019`'s argument against a tombstone is reversed in `000029`, which says so explicitly.** That
note described a lookup that is renamed and hard-deleted; a tombstone beside a hard delete really would
be "a second, quieter way to hide a row", because nothing would ever write it. The tree's own words are
preserved in `docs/backlog.md` under B10.


