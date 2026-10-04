"""The scroll-chain invariant, checked over the app's own log.

One wheel gesture must move exactly one thing: the island body under the
pointer if that body has room in the direction being scrolled, and the column
otherwise. This walks the frames the app printed and reports every frame where
the column offset moved while the body under the pointer still had room.

Scoped to the body under the POINTER, which is the whole difference between a
check that means something and one that does not. "Some island had room" is
almost always true — five islands, one cursor — so a checker phrased that way
reported 15 violations per gesture against a chain behaving exactly as
designed.

Read from stdin, which is `tail -n +N tfg.log`.
"""

import re
import sys

SCROLL = re.compile(r'scroll=([0-9.]+)')
OWNER = re.compile(r'wheel_owner=(None|"[^"]*")')
CONTENT = re.compile(r'content_h: ([0-9.]+)')
VIEWPORT = re.compile(r'viewport_h: ([0-9.]+)')
OFFSET = re.compile(r'offset: ([0-9.]+)')

# Half a point, matching chrome::BODY_SCROLL_EPSILON. Below this the body is at
# its limit and the column is entitled to the gesture.
EPSILON = 0.5


def main():
    offset = None
    owner = None
    owner_had_room = False
    violations = 0
    frames = 0

    for line in sys.stdin:
        if 'zone: ' in line:
            m = SCROLL.search(line)
            if not m:
                continue
            if offset is not None and m.group(1) != offset and owner_had_room:
                violations += 1
            offset = m.group(1)
            frames += 1
            w = OWNER.search(line)
            owner = None if not w or w.group(1) == 'None' else w.group(1)[1:-1]
            owner_had_room = False
        elif owner and 'body=Some(BodyFit' in line and owner in line:
            c, v, o = CONTENT.search(line), VIEWPORT.search(line), OFFSET.search(line)
            if c and v and o:
                content = float(c.group(1))
                viewport = float(v.group(1))
                off = float(o.group(1))
                owner_had_room = content - viewport - off > EPSILON or off > EPSILON

    print(f'{violations}/{frames}')


if __name__ == '__main__':
    main()