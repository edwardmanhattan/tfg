#!/usr/bin/env python3
"""Run one uimouse command. See uimouse.py for why this exists."""
import sys
import time

sys.path.insert(0, "/tmp/opencode")
from uimouse import BTN_LEFT, BTN_RIGHT, Mouse, send_key, send_text  # noqa: E402

argv = sys.argv[1:]
if not argv:
    raise SystemExit("no command")

m = Mouse()
try:
    cmd = argv[0]
    if cmd == "move":
        m.move_abs(float(argv[1]), float(argv[2]))
    elif cmd == "movep":
        m.move_rel(float(argv[1]), float(argv[2]))
    elif cmd == "click":
        m.button(BTN_LEFT, True)
        m.button(BTN_LEFT, False)
    elif cmd == "rclick":
        m.button(BTN_RIGHT, True)
        m.button(BTN_RIGHT, False)
    elif cmd == "wheel":
        m.wheel(float(argv[1]))
    elif cmd == "type":
        send_text(m, argv[1])
    elif cmd == "key":
        send_key(m, argv[1])
    elif cmd == "down":
        m.emit(1, __import__("uimouse").key_code(argv[1]), 1)
        m.syn()
        time.sleep(1.2)
    elif cmd == "up":
        m.emit(1, __import__("uimouse").key_code(argv[1]), 0)
        m.syn()
    elif cmd == "drag":
        # press, settle, move, settle, release. The settles are what make it
        # a drag rather than a teleport: egui needs a frame to see the press
        # before the motion and another to see the motion before the release.
        x, y = float(argv[1]), float(argv[2])
        m.button(BTN_LEFT, True)
        time.sleep(0.15)
        m.move_abs(x, y)
        time.sleep(0.2)
        m.button(BTN_LEFT, False)
    else:
        raise SystemExit(f"unknown command {cmd!r}")
finally:
    m.close()