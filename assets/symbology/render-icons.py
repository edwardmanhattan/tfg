#!/usr/bin/env python3
"""Render `src/symbology/icons_generated.rs` to an SVG contact sheet.

A SECOND renderer, and deliberately only a check: the authority on what an
icon looks like is the epaint painter in `main.rs` and, once it ships, the
headless sheet in `proto/p5-epaint`. This exists because a geometry change has
to be LOOKED AT, and looking at it needs no GPU.

Reads the generated table, not the manifest, so what it draws is exactly what
the app consumes.
"""
import re
import sys
import pathlib

ROOT = pathlib.Path(__file__).resolve().parents[2]
GEN = ROOT / "src/symbology/icons_generated.rs"

CELL = 26.0          # the drawing size, in sheet units
ZOOM = 4             # nearest-neighbour blow-up
COLS = 8
BG = "#0b1020"
INK = "#22d3ce"


def names() -> list[str]:
    src = GEN.read_text()
    return re.findall(r"^ +UnitIcon::(\w+),$", src, re.M)[:25]


def geometry() -> list[str]:
    src = GEN.read_text()
    body = src[src.index("pub const GEOMETRY"):src.index("pub const FIT")]
    return re.split(r"\n    (?=&\[)", body)[1:]


def marks(chunk: str):
    """Every point of every mark in one icon's chunk, as (kind, points)."""
    out = []
    for m in re.finditer(r"IconMark::(Fill|Stroke)\(&\[(.*?)\]\)", chunk, re.S):
        pts = [
            (float(a), float(b))
            for a, b in re.findall(r"\((-?\d+\.\d+), (-?\d+\.\d+)\)", m.group(2))
        ]
        out.append((m.group(1), pts))
    return out


def main() -> int:
    ns, gs = names(), geometry()
    rows = (len(ns) + COLS - 1) // COLS
    w, h = COLS * CELL * 2.4, rows * CELL * 2.0
    svg = [
        f"<svg xmlns='http://www.w3.org/2000/svg' width='{w:.0f}' height='{h:.0f}'>",
        f"<rect width='100%' height='100%' fill='{BG}'/>",
    ]
    for i, (name, chunk) in enumerate(zip(ns, gs)):
        cx = (i % COLS) * CELL * 2.4 + CELL
        cy = (i // COLS) * CELL * 2.0 + CELL
        svg.append(
            f"<g transform='translate({cx:.1f} {cy:.1f})'>"
            f"<rect x='{-CELL/2:.1f}' y='{-CELL/2:.1f}' width='{CELL}' height='{CELL}' "
            f"fill='none' stroke='#233'/>"
        )
        for kind, pts in marks(chunk):
            if not pts:
                continue
            d = " ".join(f"{(x - 0.5) * CELL:.3f},{(y - 0.5) * CELL:.3f}" for x, y in pts)
            if kind == "Fill":
                svg.append(f"<polygon points='{d}' fill='{INK}'/>")
            else:
                svg.append(
                    f"<polyline points='{d}' fill='none' stroke='{INK}' "
                    f"stroke-width='1.58'/>"
                )
        svg.append("</g>")
        svg.append(
            f"<text x='{cx:.1f}' y='{cy + CELL * 0.95:.1f}' fill='#8ca' "
            f"font-family='monospace' font-size='7' text-anchor='middle'>{name}</text>"
        )
    svg.append("</svg>")
    out = pathlib.Path(sys.argv[1] if len(sys.argv) > 1 else "/tmp/opencode/icons.svg")
    out.write_text("\n".join(svg))
    print(f"{out} — {len(ns)} icons")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())