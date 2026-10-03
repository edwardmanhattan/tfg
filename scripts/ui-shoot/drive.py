#!/usr/bin/env python3
"""Drive the console with ONE pointer device, under a focus guard.

Two things this exists to get right, both learned the hard way:

1. **One device for the whole interaction.** Each `uim.py` invocation creates
   and destroys its own uinput device. A sequence of invocations is a sequence
   of unrelated pointers, and focus does not reliably survive it — which is how
   a username and password ended up typed into a chat window. `Mouse` is held
   open here for the entire run.

2. **A focus guard before every keystroke.** `assert_focus()` re-reads the
   compositor's focused window and raises if it is not tfg. Keystrokes are the
   dangerous half: a click landing on the wrong window does nothing, but a
   click followed by typing puts a password somewhere it does not belong. So
   the run ABORTS rather than continuing, and it aborts before typing rather
   than after.

Not a general UI-automation tool. Coordinates are measured off a screenshot of
the real window, and every step prints what it did so a wrong click is
visible in the log rather than silently swallowed.
"""
import json
import os
import subprocess
import sys
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from uimouse import BTN_LEFT, Mouse, send_key, send_text  # noqa: E402

SHOTS = "/tmp/opencode/shots"
TFG_CLASS = "tfg command center (egui)"
PY = sys.executable


def sh(cmd, **kw):
    return subprocess.run(cmd, shell=True, capture_output=True, text=True, **kw)


def focused_window():
    r = sh("timeout 5 hyprctl activewindow -j")
    try:
        return json.loads(r.stdout)
    except Exception:
        return {}


def assert_focus(where):
    """Refuse to send input to anything but tfg.

    The guard is deliberately noisy: it aborts the run and prints what IS
    focused, because the failure mode of skipping it is a password typed into
    a chat window.
    """
    win = focused_window()
    cls = win.get("class", "")
    if cls != TFG_CLASS:
        sys.exit(f"ABORT at {where}: focused window is {cls!r}, not tfg. "
                 f"No input sent. (title={win.get('title','')!r})")
    return win


def shoot(name):
    out = f"{SHOTS}/{name}.png"
    sh(f"timeout 20 grim -o eDP-1 {out}")
    print(f"  shot -> {out}")
    return out


def window_rect():
    """The tfg window in PHYSICAL pixels, scaled from hyprctl's logical rect."""
    r = sh("timeout 5 hyprctl clients -j")
    for c in json.loads(r.stdout or "[]"):
        if (c.get("class") or "").startswith("tfg"):
            x, y = c["at"]
            w, h = c["size"]
            return int(x * 1.25), int(y * 1.25), int(w * 1.25), int(h * 1.25)
    sys.exit("ABORT: tfg is not in the client list")


class Session:
    def __init__(self):
        assert_focus("start")
        self.m = Mouse()
        self.ox, self.oy, self.ww, self.wh = window_rect()
        print(f"window physical {self.ox},{self.oy} {self.ww}x{self.wh}")

    # -- input, all guarded -------------------------------------------------
    def pt(self, x, y):
        """Window-relative POINTS -> absolute physical pixels."""
        return int(self.ox + x * 1.25), int(self.oy + y * 1.25)

    def click(self, x, y, where=""):
        assert_focus(f"click {where}")
        px, py = self.pt(x, y)
        self.m.move_abs(px, py)
        time.sleep(0.12)
        self.m.button(BTN_LEFT, True)
        self.m.button(BTN_LEFT, False)
        time.sleep(0.35)

    def type(self, text, where=""):
        assert_focus(f"type {where}")
        send_text(self.m, text)

    def key(self, name, where=""):
        assert_focus(f"key {where}")
        send_key(self.m, name)

    def close(self):
        self.m.close()


def launch():
    """Start tfg against the local Minos and wait until it has focus.

    A freshly mapped window raises itself, which is the ONLY way to get it on
    top here: `hyprctl dispatch` is Lua-only on this compositor and the Lua
    surface is reachable only through the `__lua` binds the user's config
    routes everything through.
    """
    sh("pkill -x tfg")
    time.sleep(1.5)
    env = dict(os.environ, TFG_MINOS_HOST="http://127.0.0.1:8099/api/api/v1")
    subprocess.Popen(
        ["setsid", "./target/debug/tfg"],
        cwd="/home/edward/Work/Crossnet/tfg",
        env=env,
        stdout=open("/tmp/opencode/tfg.log", "w"),
        stderr=subprocess.STDOUT,
        stdin=subprocess.DEVNULL,
        start_new_session=True,
    )
    for _ in range(60):
        time.sleep(0.5)
        if focused_window().get("class") == TFG_CLASS:
            print("tfg focused")
            time.sleep(6)  # let it sync the register and settle
            return
    sys.exit("ABORT: tfg never took focus")


# Window-relative POINTS, measured off a screenshot of the real window rather
# than assumed. The login card is centred in the viewport, so these move with
# the window; they were read off `i1_login.png` at 62% and converted through
# the monitor's 1.25 scale.
LOGIN = {"identifier": (385, 436), "password": (385, 469), "signin": (385, 503)}


def sign_in(user, pwd):
    """Sign in by KEYBOARD.

    The pointer is not usable for this: libinput puts a relative pointer
    through its acceleration curve, so a delta of 100 arrives as 200, and the
    absolute-axis route (UI_ABS_SETUP before create) did not take on this
    kernel either. Neither failure is visible as an error — the compositor
    tracks the pointer happily and the clicks land in the wrong place.

    egui tabs between widgets in registration order, and the login card is
    identifier, password, Sign in — so three Tabs and an Enter is the whole
    form. That also means the pointer is not needed for anything that has a
    sensible tab order, which is most of what has to be reached.
    """
    s = Session()
    shoot("i1_login")
    s.key("Tab", where="focus identifier")
    s.type(user, where="identifier")
    shoot("i1b_identifier")
    s.key("Tab", where="focus password")
    s.type(pwd, where="password")
    shoot("i1c_password")
    s.key("Tab", where="focus sign in")
    s.key("Return", where="sign in")
    # The REST round trip is off-thread, so the card stays up for a beat.
    time.sleep(6)
    shoot("i2_after_login")
    s.close()


if __name__ == "__main__":
    cmd = sys.argv[1] if len(sys.argv) > 1 else "login"
    if cmd == "login":
        launch()
        sign_in(sys.argv[2] if len(sys.argv) > 2 else "supersuser",
                sys.argv[3] if len(sys.argv) > 3 else "tfg-dev-password")
        print("done")