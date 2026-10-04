#!/usr/bin/env python3
"""Locate the widest egui button in a screenshot by its fill colour.

The reason this exists: a hard-coded click coordinate is only correct for the
exact scroll position and content the run before it happened to leave behind.
The first execution of verify-scroll.sh clicked (101,520) believing it was the
composer's button; the app was in Readiness by then, that coordinate was
"declare ready", and the session advanced to Preparation under the script. Four
later assertions were measuring a state nobody asked for.

A button's FILL is a constant (tokens::BUTTON_GRAPHITE, #3C3C3C) while its
label is ink on top of it, so the widest horizontal run of that colour on a row
is a button's interior and the centre of that run is a point guaranteed to hit
it. Which button it is comes from the geometry, not from a guess.

    find-buttons.py shot.png [x_offset y_offset]
"""
import collections
import subprocess
import sys

GRAPHITE = (0x3C, 0x3C, 0x3C)
TOLERANCE = 3
MIN_SPAN = 40
MIN_ROWS = 8

# How wide a hole in the fill still counts as inside the button.
#
# A button's label is ink ON the fill, so the fill is not one contiguous run:
# "Scenario composer" cuts it into a dozen pieces. The first version of this
# script measured the LONGEST contiguous run and so measured only the strip of
# fill to the left of the first letter — which is why "Scenario composer" and
# "Enter preparation" both came out 134 wide and the composer could not be told
# from the button above it.
#
# Ten points is comfortably more than an inter-glyph gap and comfortably less
# than a gap between two controls.
GAP = 10


def pixels(path):
    out = subprocess.run(
        ['magick', path, '-depth', '8', 'txt:-'],
        capture_output=True, text=True, check=True,
    ).stdout
    for line in out.splitlines():
        if line.startswith('#'):
            continue
        head, _, rest = line.partition('#')
        xy, _, _ = head.partition(':')
        try:
            x, y = (int(v) for v in xy.split(',')[:2])
        except ValueError:
            continue
        h = rest[:6]
        if len(h) < 6:
            continue
        yield x, y, (int(h[0:2], 16), int(h[2:4], 16), int(h[4:6], 16))


def main():
    path = sys.argv[1]
    ox = int(sys.argv[2]) if len(sys.argv) > 2 else 0
    oy = int(sys.argv[3]) if len(sys.argv) > 3 else 0

    rows = collections.defaultdict(list)
    for x, y, rgb in pixels(path):
        if all(abs(rgb[i] - GRAPHITE[i]) <= TOLERANCE for i in range(3)):
            rows[y].append(x)

    bands = []
    for y in sorted(rows):
        xs = sorted(rows[y])
        # The whole button, holes included. A label is ink on the fill, so the
        # fill arrives as a run of fragments and the button is their envelope,
        # not the largest piece of it.
        spans = []
        start = prev = None
        for x in xs:
            if start is None:
                start = prev = x
            elif x - prev <= GAP:
                prev = x
            else:
                spans.append((start, prev))
                start = prev = x
        if start is not None:
            spans.append((start, prev))
        for a, b in spans:
            if b - a + 1 >= MIN_SPAN:
                bands.append((y, a, b))

    groups = []
    for b in bands:
        if groups and b[0] - groups[-1][-1][0] <= 2:
            groups[-1].append(b)
        else:
            groups.append([b])

    found = []
    for g in groups:
        ys = [b[0] for b in g]
        if ys[-1] - ys[0] + 1 < MIN_ROWS:
            continue
        x0 = min(b[1] for b in g)
        x1 = max(b[2] for b in g)
        found.append({
            'y0': ys[0] + oy, 'y1': ys[-1] + oy,
            'x0': x0 + ox, 'x1': x1 + ox,
            'cx': (x0 + x1) // 2 + ox, 'cy': (ys[0] + ys[-1]) // 2 + oy,
            'w': x1 - x0 + 1, 'h': ys[-1] - ys[0] + 1,
        })

    # TAB-separated fields, widest first, so a caller can take the head of the
    # list without parsing prose. The prose form is what this printed first and
    # it made every consumer carry an awk expression to recover the numbers.
    for f in sorted(found, key=lambda f: (-f['w'], f['y0'])):
        print(
            f"{f['y0']}\t{f['y1']}\t{f['x0']}\t{f['x1']}\t"
            f"{f['w']}\t{f['h']}\t{f['cx']}\t{f['cy']}"
        )
    if not found:
        print('no button found', file=sys.stderr)
        sys.exit(1)


if __name__ == '__main__':
    main()