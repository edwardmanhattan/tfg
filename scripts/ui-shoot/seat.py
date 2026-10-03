#!/usr/bin/env python3
"""Seat people in a game, so the in-game permissions become reachable.

The scenario book is gated on GAME ROLE permissions, not application ones —
`/games/scenarios` update is granted to the Game Master and one other role,
and the `inGame` middleware's administrative path needs
`/system/game-content` read, which a stock Administrator does not hold. So
authoring scenarios means actually being in the exercise, which is also what
gives the Player Picker a roster to show.
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


def login(u="supersuser", p="tfg-dev-password"):
    st, d = call("/auth/login", {"identifier": u, "password": p})
    if st != 200:
        sys.exit(f"login {u} failed {st} {d}")
    return d["data"]["access_token"]


def roles(tok, gid):
    st, d = call(f"/games/{gid}/roles", None, tok)
    if st != 200:
        return []
    rows = d.get("data") or []
    if isinstance(rows, dict):
        rows = rows.get("game_roles") or rows.get("data") or []
    return rows


def main():
    gid = int(sys.argv[1]) if len(sys.argv) > 1 else 1
    tok = login()
    me = call("/users/me", None, tok)[1].get("data", {})
    print(f"me id={me.get('id')} {me.get('username')}")

    rl = roles(tok, gid)
    print("game roles:", [(r.get("id"), r.get("name")) for r in rl][:8])
    gm = next((r for r in rl if r.get("name") == "Game Master"), None)
    if gm is None:
        sys.exit("no Game Master role in this game")
    gm_id = gm["id"]

    st, d = call(f"/games/{gid}/participants",
                 {"id_user": me.get("id"), "id_game_role": gm_id,
                  "call_sign": "SUPER-1"}, tok)
    print(f"seat me as Game Master: {st} {json.dumps(d)[:200]}")

    st, d = call(f"/games/{gid}/participants", None, tok)
    rows = d.get("data") or []
    print("roster now:", [(r.get("user_name"), r.get("role_name")) for r in rows]
          if isinstance(rows, list) else d)

    # Scenario book, now that the Game Master seat exists.
    for title, desc, steps in [
        ("First light", "the opening move",
         [("sweep the northern approaches", "0600", "0700"),
          ("establish the barrier", "0700", "0930")]),
        ("Second light", "",
         [("screen the traffic lanes", None, None)]),
    ]:
        s, d = call(f"/games/{gid}/scenarios",
                    {"title": title, "description": desc}, tok)
        if s not in (200, 201):
            print(f"scenario {title!r}: {s} {json.dumps(d)[:160]}")
            continue
        sid = d["data"]["id"]
        for content, a, b in steps:
            body = {"content": content}
            if a and b:
                body["start_hour"], body["end_hour"] = a, b
            call(f"/games/{gid}/scenarios/{sid}/steps", body, tok)
        print(f"scenario {title!r} id={sid} steps={len(steps)}")

    s, d = call(f"/games/{gid}/scenarios", None, tok)
    rows = d.get("data") or []
    if isinstance(rows, dict):
        rows = rows.get("scenarios") or []
    if isinstance(rows, list):
        for sc in rows:
            print(f"  readback {sc.get('title')!r} steps={len(sc.get('steps') or [])}")


if __name__ == "__main__":
    main()