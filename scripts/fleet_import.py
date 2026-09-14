#!/usr/bin/env python3
"""Generate assets/catalog.json + assets/fleet.json from the TNI AL fleet sheet.

Source: ~/Documents/TFG/Aset Kapal TNI AL.xlsx, sheet 1 (125 active hulls,
Sept 2026). Stdlib only; parses the .xlsx XML by cell reference (tail rows
skip column AA, so positional parsing would misalign).

Mapping (confirmed with the user):
- Sheet Kategori+Kelas -> catalog Class, all Category=Ship.
- Hulls (Nama Kapal + Nomor Lambung) -> fleet.json GameUnit seeds.
- Sim stats: required speed_kn (max, the order-cap mechanic); optional
  cruise_kn, range_nm, endurance_days, crew, disp_full_t, loa_m.
  Weapons/sensors/propulsion prose stays display-only in fleet seeds.
- Home-base coords are administrative (dermaga), not live positions.

Usage: python3 scripts/fleet_import.py [xlsx_path]
Writes to assets/ relative to the repo root (this file's parent's parent).
"""
import collections
import json
import pathlib
import re
import sys
import unicodedata
import zipfile
import xml.etree.ElementTree as ET

NS = "{http://schemas.openxmlformats.org/spreadsheetml/2006/main}"

# Column letters of interest (1-indexed sheet, header row 2).
COLS = {
    "kategori": "B", "kelas": "C", "nama": "D", "hull": "E",
    "role": "F", "origin": "G", "year": "H",
    "disp_std": "J", "disp_full": "K", "loa": "L",
    "speed_max": "P", "speed_cruise": "Q", "rng": "R",
    "endurance": "S", "crew": "T",
    "satuan": "AB", "pangkalan": "AC", "lokasi": "AD",
    "lat": "AE", "lon": "AF",
}


def col_ref(cell_ref):
    return "".join(filter(str.isalpha, cell_ref))


def num_token(raw):
    """First number-like token, or None for dashes/empties."""
    if raw is None:
        return None
    s = raw.strip()
    if not s:
        return None
    m = re.search(r"-?\d[\d.,]*", s)
    if not m:
        return None
    tok = m.group(0)
    if tok in ("-", "—", "–"):
        return None
    return tok


def parse_num(raw, thousands="auto"):
    """Indonesian-format number.

    thousands='auto': comma = decimal, dot = thousands ('59,5' -> 59.5).
    Bare X.YYY tokens are ambiguous ('11.300' = 11300 vs '112.735' =
    112.735), so range columns pass thousands=True and lat/lon pass
    thousands=False. Suffixes/ranges reduce to the first number
    ('11 (surface)' -> 11, '180-200' -> 180)."""
    tok = num_token(raw)
    if tok is None:
        return None
    if "," in tok:
        tok = tok.replace(".", "").replace(",", ".")
    elif thousands is True:
        tok = tok.replace(".", "")
    elif thousands == "auto" and re.fullmatch(r"\d{1,3}(\.\d{3})+", tok):
        tok = tok.replace(".", "")
    try:
        return float(tok)
    except ValueError:
        return None


def slug(text):
    norm = unicodedata.normalize("NFKD", text).encode("ascii", "ignore").decode()
    s = re.sub(r"[^a-z0-9]+", "-", norm.lower()).strip("-")
    return re.sub(r"-{2,}", "-", s)


def load_rows(xlsx_path):
    z = zipfile.ZipFile(xlsx_path)
    root = ET.fromstring(z.read("xl/worksheets/sheet1.xml"))
    out = []
    for r in root.iter(NS + "row"):
        if int(r.get("r")) < 3:  # title + header
            continue
        cells = {}
        for c in r.iter(NS + "c"):
            v = c.find(NS + "v")
            cells[col_ref(c.get("r"))] = v.text if v is not None else ""
        if not cells.get("D"):
            continue
        out.append({k: cells.get(col, "") for k, col in COLS.items()})
    return out


def mode(values):
    """Most common value; ties break to first in sheet order."""
    counts = collections.Counter(values)
    top = max(counts.values())
    for v in values:
        if counts[v] == top:
            return v
    return values[0]


def main():
    repo = pathlib.Path(__file__).resolve().parent.parent
    src = pathlib.Path(sys.argv[1]).expanduser() if len(sys.argv) > 1 else (
        pathlib.Path.home() / "Documents/TFG/Aset Kapal TNI AL.xlsx")
    rows = load_rows(src)
    print(f"data rows: {len(rows)}")

    # Group hulls by class (sheet Kelas), preserving sheet order.
    by_class = collections.OrderedDict()
    for row in rows:
        by_class.setdefault(row["kelas"], []).append(row)

    classes, fleet = [], []
    seen_ids = collections.Counter()
    for kelas, members in by_class.items():
        cid = slug(kelas)
        assert cid, f"empty class id for {kelas!r}"
        speeds = [parse_num(m["speed_max"]) for m in members]
        cruise = [parse_num(m["speed_cruise"]) for m in members]
        rng = [parse_num(m["rng"], thousands=True) for m in members]
        assert all(s is not None for s in speeds), f"{kelas}: missing max speed"
        assert all(s is not None for s in cruise), f"{kelas}: missing cruise speed"
        assert all(s is not None for s in rng), f"{kelas}: missing range"
        stats = {
            "speed_kn": mode(speeds),
            "cruise_kn": mode(cruise),
            "range_nm": mode(rng),
        }
        opt = {
            "endurance_days": mode([parse_num(m["endurance"]) for m in members]),
            "crew": mode([parse_num(m["crew"]) for m in members]),
            "disp_full_t": mode([parse_num(m["disp_full"]) for m in members]),
            "loa_m": mode([parse_num(m["loa"]) for m in members]),
        }
        for k, v in opt.items():
            if v is not None:
                stats[k] = v
        roles = list(dict.fromkeys(m["role"] for m in members if m["role"]))
        classes.append({
            "id": cid, "category": "ship", "name": kelas,
            "stats": stats, "types": roles,
        })
        for m in members:
            lat = parse_num(m["lat"], thousands=False)
            lon = parse_num(m["lon"], thousands=False)
            assert lat is not None and -11.0 <= lat <= 6.5, f"{m['nama']}: bad lat {m['lat']!r}"
            assert lon is not None and 95.0 <= lon <= 141.5, f"{m['nama']}: bad lon {m['lon']!r}"
            base = slug(f"{m['nama']} {m['hull']}")
            seen_ids[base] += 1
            fid = base if seen_ids[base] == 1 else f"{base}-{seen_ids[base]}"
            fleet.append({
                "id": fid, "name": m["nama"], "hull": m["hull"],
                "class_id": cid, "role": m["role"], "origin": m["origin"],
                "year": m["year"], "satuan": m["satuan"],
                "pangkalan": m["pangkalan"], "lokasi": m["lokasi"],
                "lat": lat, "lon": lon,
            })

    dups = [k for k, n in seen_ids.items() if n > 1]
    print(f"classes: {len(classes)}, fleet seeds: {len(fleet)}, dup ids: {dups}")
    class_ids = {c["id"] for c in classes}
    assert len(class_ids) == len(classes), "class id collision"
    assert all(f["class_id"] in class_ids for f in fleet), "orphan fleet entry"

    cat_doc = {
        "version": 1,
        "_generated": "scripts/fleet_import.py from Aset Kapal TNI AL.xlsx (Aktif, Sept 2026)",
        "categories": {
            "ship": {
                "required_stats": ["speed_kn"],
                "optional_stats": ["cruise_kn", "range_nm", "endurance_days",
                                   "crew", "disp_full_t", "loa_m"],
            },
            "plane": {"required_stats": [], "optional_stats": []},
            "tank": {"required_stats": [], "optional_stats": []},
            "port": {"required_stats": [], "optional_stats": []},
        },
        "classes": classes,
    }
    fleet_doc = {
        "version": 1,
        "_generated": "scripts/fleet_import.py from Aset Kapal TNI AL.xlsx (Aktif, Sept 2026)",
        "_note": "Home-base coords are administrative (dermaga), not live positions.",
        "units": fleet,
    }
    (repo / "assets/catalog.json").write_text(json.dumps(cat_doc, indent=2, ensure_ascii=False) + "\n")
    (repo / "assets/fleet.json").write_text(json.dumps(fleet_doc, indent=2, ensure_ascii=False) + "\n")
    print("wrote assets/catalog.json + assets/fleet.json")


if __name__ == "__main__":
    main()
