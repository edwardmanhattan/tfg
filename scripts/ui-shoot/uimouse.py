#!/usr/bin/env python3
"""A uinput pointer for a Wayland compositor.

ydotool and dotool both need root to create their device. /dev/uinput here
carries an ACL for this user, so the device can be created directly — which is
all either of those tools does anyway.

Hyprland/libinput picks the device up on creation, so nothing has to be told
about it and no compositor-side configuration is needed.

Commands (each does one thing and exits, so the caller keeps control):
  move X Y      absolute move, in compositor pixels
  movep DX DY   relative move, in compositor pixels
  click         press then release at the current position
  rclick        right press then release
  wheel N       vertical scroll, positive = away from the operator
  type TEXT     synthetic key events for ASCII text
  key NAME      one key by name, e.g. Tab, Return, Left
  down NAME     press a key and leave it down
  up NAME       release it
  drag X Y      press, move there, release — one atomic gesture
"""

import ctypes
import ctypes.util
import fcntl
import os
import struct
import sys
import time

UINPUT_PATH = "/dev/uinput"

# ioctl numbers from linux/uinput.h. Computed rather than pasted so a mistake
# shows up as a mismatch instead of silently driving the wrong bits.
_IOC_NRBITS = 8
_IOC_TYPEBITS = 8
_IOC_SIZEBITS = 14
_IOC_NRSHIFT = 0
_IOC_TYPESHIFT = _IOC_NRSHIFT + _IOC_NRBITS
_IOC_SIZESHIFT = _IOC_TYPESHIFT + _IOC_TYPEBITS
_IOC_DIRSHIFT = _IOC_SIZESHIFT + _IOC_SIZEBITS
_IOC_NONE = 0
_IOC_WRITE = 1
_IOC_READ = 2


def _ioc(direction, typ, nr, size):
    return (
        (direction << _IOC_DIRSHIFT)
        | (ord(typ) << _IOC_TYPESHIFT)
        | (nr << _IOC_NRSHIFT)
        | (size << _IOC_SIZESHIFT)
    )


BUS_VIRTUAL = 0x06

EV_SYN = 0x00
EV_KEY = 0x01
EV_REL = 0x02
EV_ABS = 0x03

ABS_X = 0x00
ABS_Y = 0x01

REL_X = 0x00
REL_Y = 0x01
REL_WHEEL = 0x08
REL_HWHEEL = 0x06

BTN_LEFT = 0x110
BTN_RIGHT = 0x111

# struct uinput_setup on THIS system is 92 bytes:
#   struct input_id id;   4 x __u16 =  8
#   char name[80];                   = 80
#   __u32 ff_effects_max;            =  4
#
# The header here carries no ff_effects_count and no absolute-axis arrays,
# unlike the fuller definition in some kernel trees. Getting this wrong is not
# a compile error — it is a silently wrong ioctl argument, and the kernel
# answers EINVAL with nothing to say which field was wrong. The size and the
# ioctl numbers were read out of a COMPILED PROBE against the real headers
# rather than transcribed by hand.
UI_DEV_SETUP_FMT = "4H80sI"
UI_DEV_SETUP_SIZE = struct.calcsize(UI_DEV_SETUP_FMT)

# The size field of a _IOW request is the STRUCT SIZE, and it was wrong first
# time round: a stale 132 made UI_DEV_SETUP 0x40845503 where the kernel
# expects 0x405c5503, and the kernel's only answer is a bare EINVAL. Computed
# from the format string so the two cannot drift apart again.
UI_SET_EVBIT = _ioc(_IOC_WRITE, "U", 100, 4)
UI_SET_KEYBIT = _ioc(_IOC_WRITE, "U", 101, 4)
UI_SET_RELBIT = _ioc(_IOC_WRITE, "U", 102, 4)
UI_DEV_SETUP = _ioc(_IOC_WRITE, "U", 3, UI_DEV_SETUP_SIZE)
# _IO, so _IOC_NONE and a zero size. Encoding these with a direction bit
# gives 0x40005501 where the kernel expects 0x00005501, and the answer is a
# bare EINVAL — which is how this was found, not by reading the header.
UI_SET_ABSBIT = _ioc(_IOC_WRITE, "U", 103, 4)
UI_ABS_SETUP = _ioc(_IOC_WRITE, "U", 4, 12)  # u16 code + u16 filler + 6x s32

UI_DEV_CREATE = _ioc(_IOC_NONE, "U", 1, 0)
UI_DEV_DESTROY = _ioc(_IOC_NONE, "U", 2, 0)

# struct input_event: timeval(2x i64) + type + code + value
EVENT_FMT = "llHHi"
EVENT_SIZE = struct.calcsize(EVENT_FMT)


# Names mapped to evdev codes. Enough for driving a UI, not a full table.
#
# The punctuation row is written out longhand because the obvious compact form
# -- enumerating from KEY_1 -- silently maps ')' and the digits onto the same
# codes, so a password containing a bracket types a digit. That is exactly the
# kind of bug that looks like "the app rejected my password".
KEYS = {
    "Return": 28, "Enter": 28, "Esc": 1, "Escape": 1, "Tab": 15,
    "Backspace": 14, "Delete": 111,
    "Left": 105, "Right": 106, "Up": 103, "Down": 108,
    "Home": 102, "End": 107, "Space": 57, "PageUp": 104, "PageDown": 109,
    "minus": 12, "equal": 13, "comma": 51, "dot": 52, "slash": 53,
    "backslash": 43, "semicolon": 39, "apostrophe": 40,
    "grave": 41, "leftbracket": 26, "rightbracket": 27,
}
SHIFT = 42
for _i, _c in enumerate("abcdefghijklmnopqrstuvwxyz"):
    KEYS[_c] = 29 + _i
for _i, _c in enumerate("0123456789"):
    KEYS[_c] = 2 + _i

# Shifted forms, spelled out for the same reason as above.
KEYS.update({
    "-": 12, "_": 12, "+": 13, "=": 13,
    "[": 26, "{": 26, "]": 27, "}": 27,
    ";": 39, ":": 39, "'": 40, '"': 40,
    "`": 41, "~": 41, "\\": 43, "|": 43,
    ",": 51, "<": 51, ".": 52, ">": 52, "/": 53, "?": 53,
    "!": 2, "@": 3, "#": 4, "$": 5, "%": 6, "^": 7, "&": 8, "*": 9,
    "(": 10, ")": 11,
})


class Mouse:
    def __init__(self):
        self.fd = os.open(UINPUT_PATH, os.O_WRONLY)
        self._setup()

    def _setup(self):
        # Via the IOCTL, not a write.
        #
        # Writing the struct is the legacy path and this kernel answers
        # EINVAL for it with nothing to say why — verified by compiling the
        # same probe in C, so it is the kernel and not this code. The
        # uinput.h comment on UI_DEV_SETUP says exactly this: it was ADDED as
        # the way to set device parameters. C: `write` -> EINVAL, `ioctl` -> 0.
        setup = struct.pack(
            UI_DEV_SETUP_FMT,
            BUS_VIRTUAL, 0x1234, 0x5678, 0x0001,
            b"tfg-shoot-mouse".ljust(80, b"\0"),
            0,                               # ff_effects_max
        )
        import fcntl
        fcntl.ioctl(self.fd, UI_DEV_SETUP, setup)

        for ev in (EV_KEY, EV_REL, EV_ABS, EV_SYN):
            self._ioc(UI_SET_EVBIT, ev)
        for axis in (ABS_X, ABS_Y):
            self._ioc(UI_SET_ABSBIT, axis)
        for rel in (REL_X, REL_Y, REL_WHEEL, REL_HWHEEL):
            self._ioc(UI_SET_RELBIT, rel)
        # EVERY key this class can send, not just the buttons.
        #
        # libinput decides a device is a keyboard from the key bits it
        # advertises at creation. Registering only BTN_LEFT/BTN_RIGHT and then
        # emitting KEY_A means the events are simply dropped — the device is a
        # mouse that occasionally makes key noises. The symptom is exact and
        # misleading: clicks land perfectly (the coordinates were verified
        # against the pixels) and no character ever appears.
        for key in {BTN_LEFT, BTN_RIGHT, SHIFT, *KEYS.values()}:
            self._ioc(UI_SET_KEYBIT, key)

        # Absolute axes, declared BEFORE the device is created.
        #
        # The order is the whole trick. The header says UI_ABS_SETUP sets the
        # ranges "for the input device to be created", and it means that
        # literally: calling it afterwards is EINVAL, from C as well as from
        # here. Getting this right is what makes positioning exact at all —
        # as a relative device the pointer goes through libinput's
        # acceleration curve, so a delta of 100 arrives as 200 and a large
        # delta saturates into a corner. Measured: asking for (100,100)
        # produced a cursor at (200,200), and asking for (1200,700) produced
        # the bottom-right corner of the screen.
        for code, hi in ((ABS_X, 1919), (ABS_Y, 1079)):
            fcntl.ioctl(self.fd, UI_ABS_SETUP,
                        struct.pack("HH6i", code, 0, 0, 0, hi, 0, 0, 0))

        fcntl.ioctl(self.fd, UI_DEV_CREATE)

        # libinput needs the device to exist before it will report a seat, and
        # a request sent before the hotplug lands is simply dropped.
        time.sleep(0.4)

    def _ioc(self, request, value):
        fcntl.ioctl(self.fd, request, value)

    def emit(self, etype, code, value):
        now = time.time()
        sec = int(now)
        usec = int((now - sec) * 1_000_000)
        os.write(self.fd, struct.pack(EVENT_FMT, sec, usec, etype, code, value))

    def syn(self):
        self.emit(EV_SYN, 0, 0)

    def move_abs(self, x, y):
        """Go to (x, y) in compositor pixels.

        Absolute, so this is the coordinate rather than a delta from wherever
        the pointer happened to be. That matters more than it sounds: as a
        relative device the pointer is put through libinput's acceleration
        curve, which is why a delta of 100 arrived as 200 and a large delta
        saturated into a screen corner.
        """
        self.emit(EV_ABS, ABS_X, max(0, min(1919, int(x))))
        self.emit(EV_ABS, ABS_Y, max(0, min(1079, int(y))))
        self.syn()
        time.sleep(0.03)

    def move_rel(self, dx, dy):
        self.emit(EV_REL, REL_X, int(dx))
        self.emit(EV_REL, REL_Y, int(dy))
        self.syn()
        time.sleep(0.03)

    def button(self, btn, down):
        self.emit(EV_KEY, btn, 1 if down else 0)
        self.syn()
        time.sleep(0.04)

    def wheel(self, n):
        self.emit(EV_REL, REL_WHEEL, int(n))
        self.syn()
        time.sleep(0.05)

    def close(self):
        try:
            fcntl.ioctl(self.fd, UI_DEV_DESTROY)
        except Exception:
            pass
        os.close(self.fd)


def key_code(name):
    if name in KEYS:
        return KEYS[name]
    if name.startswith("KEY_"):
        return KEYS.get(name[4:].lower())
    raise SystemExit(f"unknown key {name!r}")


def send_key(m, name):
    code = key_code(name)
    shift = name.isupper() or (len(name) == 1 and not name.isalnum())
    if shift:
        m.emit(EV_KEY, SHIFT, 1)
    m.emit(EV_KEY, code, 1)
    m.syn()
    time.sleep(0.03)
    m.emit(EV_KEY, code, 0)
    if shift:
        m.emit(EV_KEY, SHIFT, 0)
    m.syn()
    time.sleep(0.05)


def send_text(m, text):
    for ch in text:
        if ch == " ":
            send_key(m, "Space")
            continue
        if ch == "\n":
            send_key(m, "Return")
            continue
        if ch.upper() == ch and ch.isalpha():
            send_key(m, ch)
            continue
        send_key(m, ch.lower())