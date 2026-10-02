// Regenerate `assets/symbology/milsymbol.tsv` from the upstream MIT tables.
//
// The vendored TSV is what `generate.rs` reads; this script is what produced
// it. It is NOT part of the build and NOT part of `--symbology-generate`:
// the Rust side must stay a two-second, dependency-free, offline check, and
// requiring node plus a 8 MB checkout to review an icon is the opposite of
// that. Run this only when re-pinning upstream, and read the diff.
//
//   node assets/symbology/extract-mjs.mjs            # prints the TSV
//   node assets/symbology/extract-mjs.mjs --out x.tsv
//
// Requires a checkout of spatialillusions/milsymbol at MILSYMBOL_SRC.
//
// Why the JS at all, rather than hand-copying path strings: the tables are
// JavaScript objects built by functions, with `{...}[affiliation]` lookups and
// `ms._scale` / `ms._translate` wrappers. Extracting them by text-matching is
// how you get a table that is subtly wrong in a way nobody notices. Calling the
// real functions gets the same geometry the upstream library draws.
//
// `affiliation: "Friend"` is deliberate and is the one real judgement here. 32
// of the 1061 ground keys vary their geometry by affiliation, but they vary it
// only in SIZE: the same shape, inset to suit that frame's box, because
// upstream scales the icon into the frame's own bounding box. `generate.rs`
// normalises every icon by its own ink box, so the size difference is discarded
// and the shape survives. Taking "Friend" picks the largest, least-cropped
// variant of each. Verified: 1029 of 1061 keys are byte-identical across all
// four affiliations, and every one of the 32 that differ is a scaled copy.

import { readFileSync, writeFileSync } from "node:fs";
import { createRequire } from "node:module";

const COMMIT = "f5134380157f475cbf5a9bdec69b6c33cf66e0e7";
const SRC = process.env.MILSYMBOL_SRC;
if (!SRC) {
  console.error("set MILSYMBOL_SRC to a milsymbol checkout at " + COMMIT);
  process.exit(2);
}

// The library reads this at module scope (`ms.js:3`) and rollup replaces it at
// build time. Set it before the dynamic import or the import throws.
globalThis.__version__ = "3.0.5";
const { ms } = await import(pathToFileURL(SRC + "/src/ms.js").href);

// The two icon tables this project draws from, plus their `ms` wrapper. The
// other tables are 2525B/C letter-SIDC tables and 2525D numeric ones; the
// project implements APP-6C, and mixing generations is how you get a symbol
// set that is wrong in no particular way.
const TABLES = [
  ["ground", "/src/iconparts/ground.js", "GroundFriend"],
  ["sea", "/src/iconparts/sea.js", "SeaFriend"],
  ["air", "/src/iconparts/air.js", "AirFriend"],
  ["subsurface", "/src/iconparts/subsurface.js", "SubsurfaceFriend"],
];

const colors = {
  iconColor: {}, iconFillColor: {}, fillColor: {}, black: {}, white: {}, none: {},
};
for (const a of ["Friend", "Hostile", "Neutral", "Unknown"]) {
  colors.iconColor[a] = "#111";
  colors.iconFillColor[a] = "#111";
  colors.fillColor[a] = "#222";
  colors.none[a] = "#333";
  colors.black[a] = "#000";
  colors.white[a] = "#fff";
}

const out = {};
for (const [name, rel, frame] of TABLES) {
  const fn = (await import(pathToFileURL(SRC + rel).href)).default;
  const parts = {};
  // `STD2525: false` selects the APP-6 branch of every conditional. True
  // would give the 2525 glyphs, which are a different standard.
  fn(parts, { affiliation: "Friend", numberSIDC: false, frame,
              baseGeometry: ms._symbolGeometries[frame] },
     colors, false, "#111", false, ms);
  for (const [key, value] of Object.entries(parts)) {
    out[key] = { src: name, subpaths: marks(value) };
  }
}

function marks(value) {
  const arr = Array.isArray(value) ? value : [value];
  const res = [];
  for (const p of arr) {
    if (!p || p.type !== "path" || !p.d) continue;
    // `fill: false` means stroke-only. Upstream's DEFAULT is fill AND stroke,
    // which is how a 3-unit stroke rides on a filled shape. epaint draws a
    // fill and a stroke as separate Shapes and this symbology never wants both
    // on one mark, so a default path is recorded F and an explicit
    // `fill: false` is recorded S. This is lossy for a path that needs a hole,
    // and nothing here does: every icon in the tables that needs a hole is a
    // ring, and a ring is a stroked circle.
    res.push({ k: p.fill === false ? "S" : "F", d: p.d });
  }
  return res;
}

function pathToFileURL(p) {
  return new URL("file://" + (p.startsWith("/") ? "" : "/") + p);
}

// --- emit -------------------------------------------------------------------

const rows = [];
for (const key of Object.keys(out).sort()) {
  const { src, subpaths } = out[key];
  if (subpaths.length === 0) continue;
  // Skip keys the tables mark with a trailing space: they collide with the
  // unspaced key and one of the two silently wins upstream.
  if (key !== key.trim()) continue;
  rows.push([key, src, subpaths.map((s) => `${s.k} ${s.d}`).join(" ; ")]);
}

let tsv = `# NATO APP-6C icon geometry, extracted from spatialillusions/milsymbol.
# MIT, Copyright (c) 2017 Mans Beckman. See licenses/milsymbol-LICENSE.md.
#
# GENERATED by assets/symbology/extract-mjs.mjs -- do not hand-edit.
# Upstream: https://github.com/spatialillusions/milsymbol @ ${COMMIT} (v3.0.5)
#
# Space: a 200x200 box, y DOWN, origin at the icon centre (100,100). This is
# upstream's authoring space and is NOT tfg's em box; generate.rs converts.
#
# Grammar: real SVG path data, commands M L H V C S Q T A Z, absolute and
# relative, with implicit repeats. Curves are flattened to polylines at
# generation time.
#
# Each subpath is prefixed F (closed, filled) or S (open, stroked), which is
# upstream's fill flag. A subpath may itself be several M-separated loops,
# because epaint has no even-odd fill rule and cannot express a hole.
#
# variant-key <TAB> source-table <TAB> subpaths
`;
for (const [key, src, subs] of rows) tsv += `${key}\t${src}\t${subs}\n`;

const dest = process.argv.includes("--out")
  ? process.argv[process.argv.indexOf("--out") + 1]
  : null;
if (dest) writeFileSync(dest, tsv);
else process.stdout.write(tsv);
console.error(`${rows.length} geometry keys from ${TABLES.length} tables`);
