#!/usr/bin/env python3
"""Drive Minos into a state where all three modals have something to show.

The three modals need different things and the client will not render them
meaningfully without them:

  Scenario Composer — a game with scenarios and steps
  Fleet Picker      — a game with pieces, and register hulls to assign
  Player Picker     — a game with seated participants and a role to change

A game is born in `planning`, which is the state the whole Planning column is
for, so one game in planning is enough for all three.
"""
import json
import sys
import urllib.error
import urllib.request

BASE = "http://127.0.0.1:8099/api/api/v1"


def call(path, body=None, token=None, method=None):
    data = json.dumps(body).encode() if body is not None else None
    req = urllib.request.Request(f"{BASE}{path}", data=data, method=method)
    req.add_header("Content-Type", "application/json")
    if token:
        req.add_header("Authorization", f"Bearer {token}")
    try:
        with urllib.request.urlopen(req, timeout=20) as r:
            return r.status, json.loads(r.read() or b"{}")
    except urllib.error.HTTPError as e:
        return e.code, json.loads(e.read() or b"{}")
    except Exception as e:  # noqa: BLE001
        return 0, {"error": str(e)}


def login():
    st, d = call("/auth/login", {"identifier": "supersuser",
                                "password": "tfg-dev-password"})
    if st != 200:
        sys.exit(f"login failed {st} {d}")
    return d["data"]["access_token"]


def main():
    tok = login()
    name = sys.argv[1] if len(sys.argv) > 1 else "SHOOT-01"

    st, d = call("/games", {"name": name, "mode": "maneuver",
                            "description": "a session for photographing the console",
                            "purpose": "verify the three modals against real data",
                            "target": "the exercise area"}, tok)
    if st not in (200, 201):
        sys.exit(f"create game failed {st} {d}")
    g = d["data"]
    gid = g["id"]
    print(f"game {gid} {g['name']} state={g.get('state')}")

    # Two scenarios with steps, so the composer's right column has content
    # and its "empty book" branch is not what gets photographed.
    for st_title, desc, steps in [
        ("First light", "the opening move",
         [("sweep the northern approaches", "0600", "0700"),
          ("establish the barrier", "0700", "0930")]),
        ("Second light", None,
         [("screen the traffic lanes", None, None)]),
    ]:
        body = {"title": st_title, "description": desc or ""}
        s, d = call(f"/games/{gid}/scenarios", body, tok)
        if s not in (200, 201):
            print(f"  scenario {st_title}: {s} {d}")
            continue
        sid = d["data"]["id"]
        for content, a, b in steps:
            sb = {"content": content}
            if a and b:
                sb["start_hour"], sb["end_hour"] = a, b
            s2, d2 = call(f"/games/{gid}/scenarios/{sid}/steps", sb, tok)
            if s2 not in (200, 201):
                print(f"    step {content[:20]}: {s2} {d2}")
        print(f"  scenario {st_title!r} id={sid} steps={len(steps)}")

    # A second player to seat, so the Player Picker has a directory with more
    # than one row and a roster that can change a role.
    for uname in ("operator1", "operator2"):
        s, d = call("/users", {"username": uname, "name": uname.title(),
                               "email": f"{uname}@tfg.domain",
                               "password": "tfg-dev-password"}, tok)
        print(f"  user {uname}: {s}")

    s, d = call(f"/games/{gid}", None, tok)
    print("verify:", s, d.get("data", {}).get("state"))
    print(f"GAME_ID={gid}")


if __name__ == "__main__":
    main()