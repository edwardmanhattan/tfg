#!/usr/bin/env python3
"""Seed a unit catalogue into local Minos, so the Fleet picker has rows to drag.

The two drag-to-map gestures could not be verified because the local mirror has
no hulls. This is why, and it is not a console bug:

    synced 131 rows: ... unit_types 19, unit_classes 0, units 0, ...

Nineteen types and zero classes. `POST /units` requires `id_unit_class`, and a
class requires `id_unit_type`, so the chain is type -> class -> hull and it has
to be walked in that order. Nothing seeds it: the migrator has four subcommands
(`up`, `down`, `version`, `force`, `seed-superuser`) and none of them touch the
taxonomy. So this script does.

IDEMPOTENT, keyed on name. A rerun finds the existing class and hull and leaves
them alone, so it can be run against a database that already has some of this.
That matters because the console's own sync is a mirror: rows it has already
mirrored would otherwise be duplicated, and a Fleet picker with two hulls named
identically is not a thing to photograph.

The three hierarchy writes below are a second reason this exists. A hull in the
register is not in an exercise: `PUT /games/{id}/units/{unit_id}/hierarchy`
places it, and `.../placement` puts it on the map. Without those the Fleet
island says "No pieces yet" even with a full register, which reads as a console
defect and is not one.

    python3 scripts/ui-shoot/seedunits.py            # 6 hulls across 3 classes
    python3 scripts/ui-shoot/seedunits.py --count 12
"""

import argparse
import json
import sys
import urllib.error
import urllib.request

BASE = "http://127.0.0.1:8099/api/api/v1"
IDENTIFIER = "supersuser"
PASSWORD = "tfg-dev-password"

# (Minos unit type id_name, fallback category, CLASS NAME, class symbol, [hull names])
#
# LOOKED UP BY id_name, never by a numeric id. The 19 types the server ships
# have their own names, and the first guess at them -- `frigate`, `destroyer`,
# `amphibious_ship` -- matched nothing: `id_name` on a seeded type is a display
# string (`Frigat`, `Korvet`, `Kapal Pendarat Tank`), not a slug. A script that
# finds no types reports zero hulls seeded and exits zero, which is the worst
# combination available, so the lookup falls back to the type's CATEGORY, which
# is English and stable.
#
# THE CLASS NAMES ARE THE CONSOLE'S OWN, and this is why the drag gestures were
# unreachable for so long. The console resolves a register hull's simulation
# statistics by MATCHING ITS CLASS NAME against the local catalogue
# (`catalog::find_class_by_name`, case-insensitively), and a name that is not in
# there means placement refuses:
#
#     "place refused: no sim stats for <name>"
#
# The catalogue is assets/catalog.json -- 41 named hulls, "Ahmad Yani / Van
# Speijk", "Makassar / LPD" and so on. The first pass invented class names --
# "Frigate class", "Corvette class" -- which are honest descriptions and match
# nothing, so every row in the picker duly read "no sim stats".
#
# So the class name here is a JOIN KEY, not a label. Worth saying plainly,
# because it is the kind of coupling a future reader would otherwise tidy by
# renaming one side and silently breaking placement.
#
# The three chosen are a frigate, a corvette and a landing ship, which is
# enough for the gesture: the picker's first click is a branch tab and the drag
# starts from a row in the column beside it.
CATALOGUE = [
    (
        "Frigat",
        "Frigate",
        "Ahmad Yani / Van Speijk",
        "FF",
        ["KRI Ahmad Yani", "KRI Sultan Iskandar Muda"],
    ),
    (
        "Korvet",
        "Corvette",
        "Bung Tomo / Meko 200",
        "FS",
        ["KRI Bung Tomo", "KRI John Lie"],
    ),
    (
        "Kapal Pendarat Tank",
        "Amphibious",
        "Makassar / LPD",
        "LPD",
        ["KRI Makassar", "KRI Developersih"],
    ),
]

class Minos:
    def __init__(self):
        self.token = self.login()

    def login(self):
        body = json.dumps({"identifier": IDENTIFIER, "password": PASSWORD}).encode()
        req = urllib.request.Request(
            f"{BASE}/auth/login",
            data=body,
            headers={"Content-Type": "application/json"},
        )
        with urllib.request.urlopen(req) as r:
            return json.load(r)["data"]["access_token"]

    def get(self, path):
        req = urllib.request.Request(f"{BASE}{path}", headers=self._auth())
        with urllib.request.urlopen(req) as r:
            return json.load(r)["data"]

    def post(self, path, payload):
        """POST, trying the `payload` form part first and a JSON body second.

        The two routes this script writes to disagree, and they are written the
        same way. `UnitController.Create` reads `c.FormValue("payload")`, and
        the comment above `uploadPayloadField` explains at length why the fields
        of a write-with-a-picture travel as one JSON document in a form part.
        Measured against the running server:

            POST /units          multipart `payload`  -> 201
            POST /units          plain JSON body     -> 400 "needs a `payload` part"
            POST /unit-classes   multipart `payload`  -> 400, every field "missing"
            POST /unit-classes   plain JSON body     -> 201

        Same handler shape, same field name, opposite requirements. The contract
        declares `application/json` for both, which is right for neither of the
        two that matter here.

        Rather than encode one guess and be wrong on half the writes, try the
        documented shape and fall back, and report both refusals when neither
        works so the next reader is not guessing which one was tried.
        """
        retry_statuses = (400, 422)
        boundary = "----tfgseed"
        body = (
            f"--{boundary}\r\n"
            'Content-Disposition: form-data; name="payload"\r\n'
            "Content-Type: application/json\r\n\r\n"
        ).encode() + json.dumps(payload).encode() + f"\r\n--{boundary}--\r\n".encode()
        try:
            return self._send(
                "POST", path, None,
                data=body,
                content_type=f"multipart/form-data; boundary={boundary}",
                retry_statuses=retry_statuses,
            )
        except SystemExit as multipart_refused:
            if getattr(multipart_refused, "code", 0) not in retry_statuses:
                raise
            # FALL BACK ON ANY 4xx, not on one naming `payload`. The three
            # routes disagree in how they say so: /units names the missing part,
            # /unit-classes names every field as missing, and
            # /games/{id}/units names its two required fields — three different
            # messages for the same cause, so matching on the wording picks two
            # routes and misses the third.
            try:
                return self._send("POST", path, payload)
            except SystemExit as json_refused:
                # The SECOND shape's status is the one that matters: the
                # multipart 400 is a foregone conclusion, and reporting only the
                # combined message loses the 409 a caller needs to tell "already
                # assigned" from "refused".
                both = SystemExit(
                    f"both shapes refused POST {path}\n"
                    f"  multipart : {multipart_refused}\n"
                    f"  json body : {json_refused}"
                )
                both.code = getattr(json_refused, "code", 0)
                raise both from None

    def _send(self, method, path, payload, data=None, content_type=None,
              retry_statuses=()):
        """One request. A 4xx raises SystemExit carrying Minos's own answer.

        Minos names the offending field in `data.errors`, and dropping that turns
        "you sent the wrong shape" into a stack trace. Echoing it is the
        difference between a script that can be fixed and one that has to be
        rewritten.

        `retry_statuses` is for the caller that wants to try another encoding:
        a 403 or a 409 is a real answer and must not be retried into a different
        question.
        """
        req = urllib.request.Request(
            f"{BASE}{path}",
            data=data if data is not None else json.dumps(payload).encode(),
            headers={
                **self._auth(),
                "Content-Type": content_type or "application/json",
            },
            method=method,
        )
        try:
            with urllib.request.urlopen(req) as r:
                return json.load(r)["data"]
        except urllib.error.HTTPError as e:
            body = e.read().decode(errors="replace")[:400]
            failure = SystemExit(f"{method} {path} -> {e.code}\n{body}")
            failure.code = e.code
            raise failure from None

    def put(self, path, payload):
        return self._send("PUT", path, payload)

    def _auth(self):
        return {"Authorization": f"Bearer {self.token}"}


def rows_of(data):
    """The list, whatever the envelope is.

    Minos is not consistent about this and the console learned that the hard way
    (`parse_roster`): some routes return a bare array, some `{key: [...]}`, some
    `{data: [...]}`. Every caller here normalises rather than assumes.
    """
    if isinstance(data, list):
        return data
    if isinstance(data, dict):
        for key in ("data", "units", "unit_classes", "unit_types", "games", "results"):
            v = data.get(key)
            if isinstance(v, list):
                return v
            if isinstance(v, dict):
                for inner in v.values():
                    if isinstance(inner, list):
                        return inner
        for v in data.values():
            if isinstance(v, list):
                return v
    return []


def find_by(rows, field, value):
    for r in rows:
        if r.get(field) == value:
            return r
    return None


def reference_ids():
    """The ids every hull needs, read rather than assumed.

    ALL FROM `/helpers`, which is the only route serving the whole vocabulary.
    There is no `/unit-statuses` and no `/movement-domains`: those were the
    first guesses and they 404, because the taxonomy tables are helpers rather
    than resources of their own. `/helpers` returns them keyed by table name,
    which is what the console's own sync reads too.
    """
    m = Minos()
    helpers = m.get("/helpers")["helpers"]

    def by_name(table, want):
        for row in helpers.get(table) or []:
            if row.get("name") == want:
                return row["id"]
        return None

    types = rows_of(m.get("/unit-types?page_size=200&page_number=1"))
    branches = helpers.get("service_branches") or []
    return m, {
        # BY NAME, NOT FIRST ROW. `/helpers` lists the movement domains as Air,
        # Land, Submerged, Surface, so "the first one" is Air and every hull
        # created with it is a ship that moves through the air. No route serves
        # them in a useful order, so the name is the contract here.
        "status": by_name("unit_statuses", "Active"),
        "user_status": by_name("user_statuses", "Active"),
        "domain": by_name("movement_domains", "Surface"),
        # TRIED IN ORDER, because the server refuses a hull whose branch does not
        # operate its class's category and no route publishes which branch
        # operates which. seed_hulls walks this once per class and caches the
        # branch that was accepted.
        "branches": [b["id"] for b in branches],
        "branch_names": {b["id"]: b.get("name") for b in branches},
        "types": {t.get("id_name"): t["id"] for t in types},
        "types_by_category": {
            (t.get("category") or {}).get("name"): t["id"] for t in types
        },
    }



def seed_classes(m, ref):
    existing = {c["name"]: c for c in rows_of(m.get("/unit-classes?page_size=200&page_number=1"))}
    made = {}
    for type_id_name, category, class_name, symbol, _hulls in CATALOGUE:
        type_id = ref["types"].get(type_id_name)
        if type_id is None:
            type_id = ref["types_by_category"].get(category)
            if type_id is not None:
                print(f"  ~ no type {type_id_name!r}, using the {category!r} type {type_id}")
        if type_id is None:
            print(f"  ! no unit type for {class_name!r} by name or category, skipping")
            continue
        if class_name in existing:
            made[class_name] = existing[class_name]["id"]
            print(f"  = class {class_name!r} exists as {existing[class_name]['id']}")
            continue
        created = m.post(
            "/unit-classes",
            {"id_unit_type": type_id, "name": class_name, "id_name": symbol},
        )
        made[class_name] = created["id"]
        print(f"  + class {class_name!r} as {created['id']} (type {type_id})")
    return made


def seed_hulls(m, ref, classes):
    """Create the hulls, resolving each class's service branch by trying.

    The branch search is here rather than in the catalogue because the refusal
    is per hull and no route publishes the mapping:

        "This service branch does not operate hulls of unit class 1. A hull's
        branch must own the category its class belongs to"

    which is a good answer and only reachable by trying. So try, cache the
    branch that was accepted per class, and the search costs the number of
    classes rather than the number of hulls.
    """
    rows = rows_of(m.get("/units?page_size=200&page_number=1"))
    existing = {u["name"]: u for u in rows}
    # Hull numbers are UNIQUE among live hulls, and the first pass numbered them
    # `DD-{1000 + len(made)}` where `made` counts only this run's creations. So a
    # rerun restarted at DD-1000 and collided with the batch before it:
    #
    #   "A live hull already uses that hull number. Retire it first, or choose
    #    another."
    #
    # which is the server right and the numbering wrong. Collect what is taken
    # and hand out the next free one.
    taken = {u.get("hull_number") for u in rows if u.get("hull_number")}

    def next_hull_number():
        n = 1000
        while f"DD-{n}" in taken:
            n += 1
        taken.add(f"DD-{n}")
        return f"DD-{n}"

    made = []
    branch_for_class = {}
    for _tn, _cat, class_name, _sym, hulls in CATALOGUE:
        class_id = classes.get(class_name)
        if class_id is None:
            continue
        for hull in hulls:
            if hull in existing:
                made.append(existing[hull]["id"])
                print(f"  = hull {hull!r} exists as {existing[hull]['id']}")
                continue
            candidates = (
                [branch_for_class[class_id]]
                if class_id in branch_for_class
                else ref["branches"]
            )
            refused = None
            for branch_id in candidates:
                payload = {
                    "id_unit_class": class_id,
                    "id_service_branch": branch_id,
                    "id_movement_domain": ref["domain"],
                    "id_unit_status": ref["status"],
                    "name": hull,
                    "hull_number": next_hull_number(),
                    "source_country": "ID",
                }
                try:
                    created = m.post("/units", payload)
                except SystemExit as e:
                    if "id_service_branch" not in str(e):
                        raise
                    refused = e
                    continue
                branch_for_class[class_id] = branch_id
                made.append(created["id"])
                print(
                    f"  + hull {hull!r} as {created['id']} in branch"
                    f" {ref['branch_names'].get(branch_id, branch_id)}"
                )
                break
            else:
                print(f"  ! no branch operates {class_name!r}")
                print(f"      {refused}")
    return made


# Commanders, one per hull, because the server says so:
#
#   "That person already commands the unit \"KRI Sultan Iskandar Muda\" in this
#    exercise. One person commands one thing — a unit or a task group."
#
# which is a good rule and means a register of six hulls needs six accounts. The
# seeded database has exactly one (supersuser), so the first pass assigned one
# hull and refused five with a message that reads like a bug in this script.
CREW = ["laksamana", "kapten", "letnan", "serangut", "koperal", "bintara"]


def seed_crew(m, ref, gid):
    """Accounts to command the hulls with, one per hull, seated in the exercise.

    Created through the API rather than the database, so they are real accounts
    with real passwords and the console's own directory lists them.

    SEATED, because the server says so:

        "that user is not a participant of this game"

    Seating is what the Player picker does, and `seat.py` does it for the Game
    Master. Doing it here is not pre-empting the Player picker's own drag — that
    gesture assigns a SEATED person to a piece, and it needs a seat to assign
    from — it is the prerequisite that makes both gestures reachable.

    Role: Commando, which is the role the role list names for somebody who moves
    a piece.
    """
    existing = {r.get("username"): r["id"] for r in rows_of(m.get("/users?page_size=200&page_number=1"))}
    made = []
    for name in CREW:
        if name in existing:
            made.append((name, existing[name]))
            print(f"  = crew {name!r} exists as {existing[name]}")
            continue
        try:
            created = m.post(
                "/users",
                {
                    "username": name,
                    "name": name.capitalize(),
                    "email": f"{name}@tfg.local",
                    # 12 characters minimum, enforced by the DTO. Not a real
                    # credential: these accounts exist to be commanded with.
                    "password": "tfg-dev-password",
                    "satuan": "TNI AL",
                    # Required, and the only required field the message names.
                    # `/helpers` lists the user statuses, and "Active" is the one
                    # an account can sign in with.
                    "id_user_status": ref["user_status"],
                },
            )
        except SystemExit as e:
            print(f"  ! crew {name!r}: {e}")
            continue
        made.append((name, created["id"]))
        print(f"  + crew {name!r} as {created['id']}")

    role_id = commando_role(m, gid)
    if role_id is None:
        print("  ! no Commando role in this game; the hulls cannot be commanded")
        return made

    for name, uid in made:
        try:
            m.post(
                f"/games/{gid}/participants",
                {"id_user": uid, "id_game_role": role_id, "call_sign": name.upper()[:16]},
            )
            print(f"  + seated {name!r} as Commando")
        except SystemExit as e:
            # 409 here is "already in this game", which a rerun hits for every
            # seat it placed last time. Not a failure.
            if getattr(e, "code", 0) == 409:
                continue
            print(f"  ! seat {name!r}: {e}")
    return made


def commando_role(m, gid):
    rows = rows_of(m.get(f"/games/{gid}/roles"))
    for role in rows:
        if role.get("name") == "Commando":
            return role["id"]
    return None


def assign_to_game(m, ref, hull_ids):
    """Put the hulls in an exercise, and deliberately NOT on the map.

    Without this the Fleet island reads "No pieces yet" against a register with
    six hulls in it, and the console's own checklist says "no units assigned" as
    a blocker.

    Assignment only. Placement is the thing under test: the console answers an
    assign with "{name} assigned — click the map to place it", and the map click
    IS the drag-to-map gesture. Placing them here would pre-empt the thing this
    seeding exists to make testable.
    """
    games = rows_of(m.get("/games?page_size=100&page_number=1"))
    planning = [g for g in games if g.get("state") == "planning"]
    if not planning:
        print("  ! no game in planning, skipping assignment")
        return None
    gid = max(planning, key=lambda g: g["id"])["id"]

    print("crew:")
    crew = seed_crew(m, ref, gid)
    if len(crew) < len(hull_ids):
        print(
            f"  ! {len(crew)} accounts for {len(hull_ids)} hulls."
            " One person commands one thing, so the surplus hulls have no"
            " commander and will not appear in the Fleet island."
        )

    assigned = already = 0
    for (name, cmdr), uid in zip(crew, hull_ids):
        try:
            m.post(f"/games/{gid}/units", {"id_unit": uid, "id_commander": cmdr})
            assigned += 1
        except SystemExit as e:
            # "That hull is already in this game" is a 409, and a rerun of this
            # script hits it for every hull it placed last time. Counting it as
            # a failure would make a second run look like a broken one.
            if getattr(e, "code", 0) == 409:
                already += 1
                continue
            print(f"  ! assign hull {uid} to {name}: {e}")
    print(
        f"  assigned {assigned}, already in the game {already},"
        f" of {len(hull_ids)} hulls in game {gid} with one commander each"
    )
    print("  left unplaced on purpose: placement is the drag under test")
    return gid


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--no-assign", action="store_true",
                    help="seed the register but leave the exercise empty")
    args = ap.parse_args()

    print("reference data:")
    m, ref = reference_ids()
    print(f"  status  = {ref['status']} (Active)")
    print(f"  domain  = {ref['domain']} (Surface)")
    print(f"  branches= {[(b, ref['branch_names'].get(b)) for b in ref['branches']]}")
    print(f"  unit types: {len(ref['types'])}")

    print("classes:")
    classes = seed_classes(m, ref)

    print("hulls:")
    hulls = seed_hulls(m, ref, classes)
    print(f"  register now holds {len(hulls)} hulls from this script")

    if not args.no_assign:
        print("assignment:")
        assign_to_game(m, ref, hulls)

    print("done. Restart the console and sync the register.")


if __name__ == "__main__":
    sys.exit(main())