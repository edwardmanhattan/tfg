#!/usr/bin/env bash
# Assert things about a capture, by pixel, and exit non-zero when they are
# not true.
#
# A capture is a picture, and a picture is an opinion until something
# counts its pixels. This is that something: it answers "is there ONE rule
# between these two fields" and "is the column flush to the left edge" with
# a number, so a sweep can FAIL on a wrong pixel instead of a human
# noticing it three days later.
#
# The palette is read from the token layer at run time, so an ink that moves
# cannot leave these assertions quietly asserting about the old colour.
#
# Usage: scripts/shot-assert.sh <shot.png> <assertion> [<assertion> ...]
#
#   at:X,Y=INK          the pixel at (X,Y) matches INK
#   not:X,Y=INK         the pixel at (X,Y) does NOT match INK
#   runs:X=INK=N        exactly N contiguous runs of INK in the column at X
#   runs:X=INK>=N       at least N runs
#   runs:X=INK<=N       at most N runs
#   rows:X=INK>=N       at least N pixels of INK in the column at X
#   row:Y=INK>=N        at least N pixels of INK in the row at Y
#   cyan:X>=N           at least N cyan-ish pixels in the column at X
#   cyan:X,Y            the pixel at (X,Y) is cyan-ish
#
# INK is #RRGGBB or a name from the console's palette: cut-grey,
# radar-cyan, console-night, panel-slate, hairline-slate, signal-green,
# warning-sand, data-amber, idle-grey. An optional ~N suffix sets the
# colour distance tolerance (default 48), for pixels that are a BLEND of
# the ink and what is under it — a 1pt rule on a 1.25x screen is 1.25
# physical pixels, so one of them is a partial ink.
#
# Exit: 0 when every assertion holds, 1 when one does not, 2 on a usage
# error. Prints one line per assertion, so the failure says what was
# expected and what was found.
set -euo pipefail
cd "$(dirname "$0")/.."

exec python3 - "$@" <<'PY'
import os
import re
import subprocess
import sys

# A 1pt rule on a 1.25x screen is 1.25 physical pixels, so the rule
# straddles two of them and the outer one is a PARTIAL ink — a cut-grey
# edge measured at 70% is (70,188,213) against a pure (140,155,174). 60
# takes those in, and still keeps the console's own surfaces apart: the
# nearest pair, CONSOLE_NIGHT and PANEL_SLATE, is 33 away, and the
# nearest ink to a surface, CUT_GREY, is 175.
DEFAULT_TOL = 60.0

# --- the console's own palette, read from the token layer ------------------
def palette():
    inks = {}
    for path in ("src/view/mod.rs", "src/shell_gpui.rs"):
        if not os.path.exists(path):
            continue
        text = open(path).read()
        for name, r, g, b in re.findall(
            r"pub const (\w+): Rgba = Rgba::rgb\(0x([0-9A-Fa-f]{2}), 0x([0-9A-Fa-f]{2}), 0x([0-9A-Fa-f]{2})\);",
            text,
        ):
            key = name.lower().replace("_", "-")
            inks[key] = (int(r, 16), int(g, 16), int(b, 16))
    return inks


INKS = palette()


def parse_ink(token):
    """'cut-grey' or '#8C9BAE', with an optional '~48' tolerance."""
    tol = DEFAULT_TOL
    if "~" in token:
        token, _, t = token.partition("~")
        try:
            tol = float(t)
        except ValueError:
            raise SystemExit(f"bad tolerance in {token!r}")
    token = token.strip()
    if token.startswith("#"):
        h = token[1:]
        if len(h) == 3:
            h = "".join(c * 2 for c in h)
        if len(h) != 6 or not re.fullmatch(r"[0-9A-Fa-f]{6}", h):
            raise SystemExit(f"bad colour {token!r}")
        rgb = (int(h[0:2], 16), int(h[2:4], 16), int(h[4:6], 16))
    else:
        key = token.lower().replace("_", "-")
        if key not in INKS:
            raise SystemExit(
                f"unknown ink {token!r}; known: {', '.join(sorted(INKS))}"
            )
        rgb = INKS[key]
    return rgb, tol


def near(px, rgb, tol):
    return sum((a - b) ** 2 for a, b in zip(px, rgb)) ** 0.5 <= tol


def is_cyan(px):
    """Cyan-ish, and robust to a partial blend over a dark ground.

    The console's cyan is RADAR_CYAN (0x22D3EE): blue and green far above
    its red. A 40% blend of it over CONSOLE_NIGHT still reads that way, so
    the test is on the DIFFERENCES, not on a distance from the pure ink.
    """
    r, g, b = px
    return b - r > 45 and g - r > 25


# --- the pixels ------------------------------------------------------------
def image_size(path):
    out = subprocess.run(
        ["magick", path, "-format", "%w %h", "info:"],
        capture_output=True, text=True,
    )
    if out.returncode != 0 or not out.stdout.strip():
        raise SystemExit(f"cannot read {path}")
    w, h = out.stdout.split()[:2]
    return int(w), int(h)


def strip(path, kind, index, size):
    """The pixels of one column or one row, in order."""
    w, h = size
    if kind == "col":
        if not 0 <= index < w:
            raise SystemExit(f"column {index} is outside the {w}x{h} image")
        crop = f"1x{h}+{index}+0"
    else:
        if not 0 <= index < h:
            raise SystemExit(f"row {index} is outside the {w}x{h} image")
        crop = f"{w}x1+0+{index}"
    out = subprocess.run(
        ["magick", path, "-crop", crop, "+repage", "txt:-"],
        capture_output=True, text=True,
    )
    if out.returncode != 0:
        raise SystemExit(f"cannot crop {path}: {out.stderr.strip()}")
    px = []
    for line in out.stdout.splitlines():
        m = re.search(r"srgb\((\d+),(\d+),(\d+)\)", line)
        if m:
            px.append(tuple(int(v) for v in m.groups()))
    return px


def runs(flags):
    """How many contiguous groups of True there are."""
    n = 0
    prev = False
    for f in flags:
        if f and not prev:
            n += 1
        prev = f
    return n


# --- the assertions --------------------------------------------------------
def check(spec, path, size):
    """(ok, description) for one assertion."""
    m = re.fullmatch(r"at:(\d+),(\d+)=(.+)", spec)
    if m:
        x, y, ink = int(m.group(1)), int(m.group(2)), m.group(3)
        rgb, tol = parse_ink(ink)
        px = strip(path, "row", y, size)
        if not 0 <= x < len(px):
            return False, f"pixel ({x},{y}) is outside the image"
        got = px[x]
        ok = near(got, rgb, tol)
        return ok, (
            f"pixel ({x},{y}) is {got} — "
            + ("matches" if ok else f"is not within {tol:.0f} of {rgb}")
        )

    m = re.fullmatch(r"not:(\d+),(\d+)=(.+)", spec)
    if m:
        x, y, ink = int(m.group(1)), int(m.group(2)), m.group(3)
        rgb, tol = parse_ink(ink)
        px = strip(path, "row", y, size)
        if not 0 <= x < len(px):
            return False, f"pixel ({x},{y}) is outside the image"
        got = px[x]
        ok = not near(got, rgb, tol)
        return ok, (
            f"pixel ({x},{y}) is {got} — "
            + ("is not that ink" if ok else f"IS that ink ({rgb})")
        )

    m = re.fullmatch(r"(runs|rows):(\d+)=(.+?)(>=|<=|=)(\d+)", spec)
    if m:
        kind, x, ink, op, want = m.group(1), int(m.group(2)), m.group(3), m.group(4), int(m.group(5))
        rgb, tol = parse_ink(ink)
        px = strip(path, "col", x, size)
        flags = [near(p, rgb, tol) for p in px]
        got = runs(flags) if kind == "runs" else sum(flags)
        ok = {"=": got == want, ">=": got >= want, "<=": got <= want}[op]
        unit = "runs" if kind == "runs" else "pixels"
        return ok, f"column {x} has {got} {unit} of {rgb} — wanted {op} {want}"

    m = re.fullmatch(r"row:(\d+)=(.+?)(>=|<=|=)(\d+)", spec)
    if m:
        y, ink, op, want = int(m.group(1)), m.group(2), m.group(3), int(m.group(4))
        rgb, tol = parse_ink(ink)
        px = strip(path, "row", y, size)
        got = sum(1 for p in px if near(p, rgb, tol))
        ok = {"=": got == want, ">=": got >= want, "<=": got <= want}[op]
        return ok, f"row {y} has {got} pixels of {rgb} — wanted {op} {want}"

    m = re.fullmatch(r"cyan:(\d+)(>=|<=|=)(\d+)", spec)
    if m:
        x, op, want = int(m.group(1)), m.group(2), int(m.group(3))
        px = strip(path, "col", x, size)
        got = sum(1 for p in px if is_cyan(p))
        ok = {"=": got == want, ">=": got >= want, "<=": got <= want}[op]
        return ok, f"column {x} has {got} cyan pixels — wanted {op} {want}"

    m = re.fullmatch(r"cyan:(\d+),(\d+)", spec)
    if m:
        x, y = int(m.group(1)), int(m.group(2))
        px = strip(path, "row", y, size)
        if not 0 <= x < len(px):
            return False, f"pixel ({x},{y}) is outside the image"
        got = px[x]
        ok = is_cyan(got)
        return ok, f"pixel ({x},{y}) is {got} — " + ("is cyan" if ok else "is not cyan")

    return None, f"cannot read the assertion {spec!r}"


def main(argv):
    if len(argv) < 2:
        print(__doc__ if False else "usage: shot-assert.sh <shot.png> <assertion> ...", file=sys.stderr)
        return 2
    path = argv[1]
    specs = argv[2:]
    if not os.path.exists(path):
        print(f"no such capture: {path}", file=sys.stderr)
        return 2
    size = image_size(path)
    failures = 0
    print(f"{path} ({size[0]}x{size[1]})")
    for spec in specs:
        ok, why = check(spec, path, size)
        if ok is None:
            print(f"  ERROR  {spec}: {why}")
            failures += 1
        elif ok:
            print(f"  ok     {spec}  — {why}")
        else:
            print(f"  FAIL   {spec}  — {why}")
            failures += 1
    print(f"{len(specs) - failures}/{len(specs)} assertions hold")
    return 1 if failures else 0


sys.exit(main(sys.argv))
PY
