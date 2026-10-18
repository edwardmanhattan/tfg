---
target: proto/p5-epaint
total_score: 15
max_score: 40
na_heuristics: 
p0_count: 2
p1_count: 2
target_identity: "file:/home/edward/Work/Crossnet/tfg/proto/p5-epaint/src/main.rs"
target_fingerprint: "sha256:73f69bb806148234fd6c5ffdfa511f36fdffcc032d54f22263c85bf573722891"
target_path: /home/edward/Work/Crossnet/tfg/proto/p5-epaint/src/main.rs
timestamp: 2026-10-09T09-18-22Z
slug: proto-p5-epaint-src-main-rs
---
⚠️ DEGRADED: single-context (subagent tool is exposed but the harness subagent depth limit blocks nested agents at depth 1; Assessment A and B were run sequentially in this context by the same model, so the two judgments are not independent).

Method: degraded single-context (A + B sequential in one context). Target: `proto/p5-epaint` — the in-flight Tern-fusion prototype (entry `src/main.rs`, panel language `src/glass.rs`, compiling the repo's real `src/tokens.rs` + `src/chrome.rs`). Fresh captures taken today from the real binary on Xvfb: `--glass-b`, `--glass-b --glass-b-shadow`, `--components`, default, `--glass-b --components`. Reference: `proto/glass-mock/` (Tern fusion mock; recommended variant B · Instrument, accent `#5B8DEF`). User complaint verbatim: "so many inconsistencies, not even close to tern ui."

## Design Health Score

| # | Heuristic | Score | Key Issue |
|---|-----------|-------|-----------|
| 1 | Visibility of System Status | 1 | No status surface in the Tern pass: no connection/WIRE state, no clock, no seat. The only readout is the sheet's debug STATE line. |
| 2 | Match System / Real World | 2 | Domain vocabulary is real (KRI hulls, SATUAN TUGAS, WIRE), but the Tern pass bodies are placeholder filler ("Body text at 15px."); the mock's Setup/Live/Closed lifecycle is absent. |
| 3 | User Control and Freedom | 1 | Tern panels are Painter draws — no close, drag, resize, or click. Escape from the component sheet is an undocumented `C` key. No undo anywhere. |
| 4 | Consistency and Standards | 1 | Three accent constants (#22D3EE, #22D3CE, #5B8DEF), six corner radii, three title treatments, two implementations of ADR-0016's rim, near-miss palette copies. |
| 5 | Error Prevention | 2 | No forms or destructive actions exist to guard; the sheet field has a focus ring but no validation, and no constraints are designed yet. |
| 6 | Recognition Rather Than Recall | 2 | Sheet controls are labeled (good), but the key-legend line ("S shadow · C components · red outline…") forces recall of developer keys; hitbox is hidden behind `H`. |
| 7 | Flexibility and Efficiency | 2 | Accelerators exist for the harness (1-3, arrows, R, H, D, S, C) but none for an operator task; variant switching is a lab control, not a user feature. |
| 8 | Aesthetic and Minimalist Design | 2 | The component sheet is disciplined craft; the frame pass ships a red debug hit-rect outline and a key-legend line by default, and two different aesthetics (warm ember vs navy) live in one binary. |
| 9 | Error Recovery | 1 | No error surface exists at all; failures have no designed presentation, naming, or recovery path. |
| 10 | Help and Documentation | 1 | No help or tooltips; the only guidance text speaks to the developer, not the operator. |
| **Total** | | **15/40** | **Poor (37.5%) — major rework before this reads as one console** |

Note: the component-sheet pass in isolation would score roughly 28–30 (its controls are labeled, focusable, and coherent); the Tern frame pass and the three-worlds problem drag the total down.

## Design Specificity Verdict

**LLM assessment.** The direction this prototype is reaching for — the warm instrument-glass register of the mock's variant B — is genuinely authored for this product: it survives contact with tfg's actual vocabulary (KRI Ahmad Yani, "8 / 125", LAUT JAWA, WIRE) and with the real operating scene (dark room, moving map behind translucent panels). That is not category-interchangeable; a generic dashboard could not wear it unchanged.

What is category-interchangeable is the current *state*. The Tern pass renders two fixed panels with one-line placeholder bodies over a map wash; the mock's B is a whole console — top band with logo, breadcrumb (Operasi Gabungan / Satuan Tugas Bravo / KRI Rigel), WIRE pill, GAME clock, seat; SESSION with numbered lifecycle steps and ratio control; ROSTER with affiliation dots and seat rows; SETTINGS with search, tab row, and setting cards; map with grid, halos, coordinate caption, scale bar. The prototype has the frame of one panel and the furniture of none. Worse, the default render includes a rust-colored debug rectangle around every panel and a key legend — it photographs as a broken build, not as a direction. And the same binary, flag-selectable, contains three different looks: the warm ember Tern world (`--glass-b`), the navy night-ops world (default variants, `--island`, `--zone`, `--modal`), and the shipped `chrome.rs` islands with mono tracked-caps titles. "Not even close to tern ui" is accurate on both counts: the Tern pass is a skeleton, and the surrounding product it is supposed to become is not the same world.

**Deterministic scan.** `impeccable detect --json proto/p5-epaint` → `[]`, exit 0: zero scannable files (the target is native Rust; the detector scans HTML/CSS markup). Clean, but contributing nothing. The factual audit below substitutes; every row is a value with file:line and its reference.

**Accent — three constants plus a typo (the user's "inconsistencies", literally):**

| Value | Defined at | Reference | Agree? |
|---|---|---|---|
| `#22D3EE` | `src/tokens.rs:58` (RADAR_CYAN), `main.rs:193` (RIM) | DESIGN.md Radar Cyan | canonical |
| `#22D3CE` | `main.rs:104` (SIGNAL) — used at `main.rs:680,690,1566` | a corrupted copy of `#22D3EE` (EE→CE) | NO — typo-drift |
| `#5B8DEF` | `glass.rs:37` (ACCENT) — used at `glass.rs:203,242,779,837,962` | mock `index.html:17` `--signal` | mock-faithful, token-hostile |

**ADR-0016 rim — implemented twice, differently:**

| Aspect | Canonical | main.rs | glass.rs |
|---|---|---|---|
| Lit rim color | `#22D3EE` (`tokens.rs:112`) | `RIM` cyan ✓ | `ACCENT` `#5B8DEF` ✗ |
| Lit rim alpha | `0.50` (`tokens.rs:113`) | `0.5` ✓ (`main.rs:788`) | full opacity ✗ (`glass.rs:203`) |
| Resting rim | CUT_GREY @ `1.0` (`tokens.rs:105-109`) | `#8C9BAE` literal @ `0.34` ✗ (`main.rs:788`) | `tokens::CUT_GREY` full ✓ |

**Corner radius — six values:**

| Value | Where | Reference |
|---|---|---|
| 6 | `tokens.rs:279` CONTROL_RADIUS ("uniform across the whole widget family") | DESIGN.md control-radius 6px |
| 8 | `main.rs:113` RADIUS | violates the uniform-6 rule |
| 12 | `tokens.rs:142` ISLAND_RADIUS (shipped islands) | shipped chrome |
| 12 / 8 | `glass.rs:659-661` tier radii | mock: cards 8px |
| 7 | `glass.rs:523` BED_RADIUS, `:799` button, `:811` wire | no reference |
| 6 | `glass.rs:837,863` checkbox, `:1002` seg button | incidental match |

**Title treatment — three:**

| Where | Treatment | Reference |
|---|---|---|
| `chrome.rs:281,457-464` (shipped) | monospace tracked caps, 12.5pt, tracking 1.8 | tokens.rs:178-179 |
| `glass.rs:237-243` (Tern frame) | proportional, untracked, 12.5pt, turns ACCENT when owning | mock `index.html:44-45`: 11px, weight 500, tracking .09em, uppercase, ink-mid; owning panel recolors the label (`:106`) — the prototype additionally lights a full 2px accent rim, spending the signal on two elements of one panel |
| `glass.rs:616-629` (sheet) | proportional + `0.09em` tracking ("the same LayoutJob `chrome.rs::tracked_caps` builds" — but that one is monospace) | mock tracking value, wrong family vs shipped rule |

**Body type:** `glass.rs:248` hardcodes `FontId::proportional(13.0)` while its caller (`main.rs:1260`) labels the line "Body text at 15px." and `tokens::TEXT_BODY_PT = 15.0`. The specimen mislabels itself by 2pt. The sheet's own controls use `tokens::TEXT_BODY_PT` (`glass.rs:796,879,934`) — so the sheet and the frame disagree inside one file.

**Scanline band — three values:** `tokens.rs:294-304` pitch 4 / alpha 9 / band = TITLE_H 30; `main.rs:837-848` band 48px tall / alpha hardcoded 17; DESIGN.md's components table says alpha 0.067 (≈17/255). The doc is stale against its own token, and the code beats both.

**Shadow — doctrine reversed between the two files:** `tokens.rs:168-173` + `main.rs:107-110` both declare hard-offset, no blur, "the first thing that stops being affordable on a software rasteriser"; `main.rs:110` then uses alpha 120 (token: 80) and offset 5×6 (token: 3×4); `glass.rs:117-153` implements a real epaint blur (blur 22, warm `#181411`) — exactly what both docs forbid, and a second answer (polygon falloff) ships beside it.

**Bevel tones:** `tokens.rs:146-148` defines STEEL_LO `#2B333E` / STEEL `#8E99A8` / STEEL_HI `#E6EBF2`; `glass.rs:161-164` re-hardcodes the same values as literals — equal today, drift by construction.

**Sheet palette vs the mock it mirrors (near-miss copies):**

| Role | Mock (`index.html`) | `glass.rs` comp | Agree? |
|---|---|---|---|
| ground | `#100E0C` (:11) | `#0E0C09` (:501) | near-miss |
| card/glass | `#1A1714` (:12) | `#2B251C` (:503) | NO — much lighter |
| ink | `#EDE6DA` (:14) | `#EDE6DA` (:509) | ✓ |
| ink-mid | `#A79D8E` (:15) | `#ADA393` (:510) | near-miss |
| ink-low | `#6B6357` (:16) | `#867C6D` (:511) | NO — much lighter |
| signal | `#5B8DEF` (:17) | `#5B8DEF` (:37) | ✓ |
| panel body fill | `rgba(26,23,20,.90)` (:174 comment) | `#1A1714 @ 0.92` (`glass.rs:179`) | ✓ |

**Debug debris on by default:** `glass.rs:255-260` draws the `#E2685F` hit-rect outline unconditionally (visible in every capture), and `main.rs:1276-1282` paints the "S shadow · C components · red outline is egui's hit rect" legend in `#A79D8E` (a mock ink-mid literal). Neither is gated.

**Wiring quirk:** `--components` sets `glass_components` (`main.rs:1754`) which is read only inside the `glass_b` branch (`main.rs:1212`); without `--glass-b` the flag is silently ignored while the window still resizes to 940×1000 (`main.rs:1718`).

## Overall Impression

Gut reaction: the best idea in the repo is being shown badly. The component sheet is real craft — the tiered cards with wires and beds are the strongest evidence yet that this register can work in egui. But the pass someone screenshots first (`--glass-b`) is two placeholder panels wearing a red debug rectangle, sitting in a binary that also contains two other finished-looking looks. The gap to the mock is not polish; it is the missing console (chrome, lifecycle, roster, settings, map furniture) plus an unresolved identity crisis over the accent. Fix the crisis, port the console, delete the debris — in that order.

## What's Working

1. **The lighting system in the component sheet** (`glass.rs:537-610`): `top_wire` / `bottom_wire` / `inset_top` / tier casts make raised and recessed read as relationships between adjacent tones rather than as painted borders. Five identical cards would be a list; the tiered containers are a design. This is the part of the direction worth keeping wholesale.
2. **Honest falsifiable experiments** (`glass.rs:90-152`): the two shadow answers (epaint blur vs polygon falloff) are shown side by side because they fail differently, and the hit-rect outline admits egui's allocation problem instead of hiding it. The prototype argues with evidence — the right instinct, wrong default state.
3. **Real product vocabulary in the demo content** (`main.rs:1496-1513`, roster with KRI hull names, "8 / 125", WIRE, coordinates): the direction is grounded in tfg's actual data, which is what keeps it from reading as a generic glass UI kit.

## Priority Issues

- **[P0] What**: Three accent constants and two rim doctrines in one binary — `#22D3EE` (`tokens.rs:58`), `#22D3CE` (`main.rs:104`, a corrupted copy), `#5B8DEF` (`glass.rs:37`); ADR-0016's lit rim is cyan@50% in `main.rs:788` and blue@full in `glass.rs:203`. **Why it matters**: the One Signal Rule only works if there is one signal. An operator cannot learn "blue means yours" while the same binary paints cyan rims in one mode and blue in another, and a resting rim at 34% alpha (`main.rs:788`) is the smudge the token doc explicitly wrote the 100% alpha to prevent. **Fix**: pick the Tern accent (`#5B8DEF`) or keep cyan — one constant per role, sourced from tokens, then delete the other two (fix the EE→CE typo, re-point `glass.rs:203` at the token with alpha 0.5). Suggested command: `/impeccable distill` (force one system) then `/impeccable polish`.
- **[P0] What**: The Tern pass ships as skeleton plus debug debris — placeholder body copy that mislabels itself ("Body text at 15px." rendered at 13px, `glass.rs:248` vs `main.rs:1260`), a rust hit-rect outline on every panel (`glass.rs:255-260`), and a key-legend line (`main.rs:1276-1282`), all ungated. **Why it matters**: the default screenshot of the direction reads as a broken build; every future review will re-pay this tax, and the mislabeled specimen actively teaches the wrong body size. **Fix**: gate the outline and legend behind the existing `--hitbox` discipline (or an `H`-style toggle like `main.rs`'s variants use), delete the placeholder lines, and render the body at `tokens::TEXT_BODY_PT` so the label is true. Suggested command: `/impeccable polish`.
- **[P1] What**: The console itself is missing from the Tern pass — no top band (connection, clock, seat), no lifecycle steps, no roster, no settings content, no map grid/caption; two fixed panels at hardcoded rects (`main.rs:1254-1255`). **Why it matters**: the mock is the spec of the target; at ~15% of its surface the prototype cannot be judged as "the new UI", only as a frame study, and the frame is already the most-finished part. **Fix**: port the mock's B layout zones into the prototype (band, SESSION steps + ratio, ROSTER seats, SETTINGS search/tabs/cards) before any further frame refinement. Suggested command: `/impeccable shape` then `/impeccable craft`-class new work on the prototype harness.
- **[P1] What**: Geometry and metric sprawl — six radii (6/7/8/12), three title treatments, band texture at 30px/α9 (tokens) vs 48px/α17 (`main.rs`) vs 0.067 (DESIGN.md), shadow alpha 120 vs token 80 and offset 5×6 vs 3×4, bevel tones re-hardcoded as literals (`glass.rs:161-164`), sheet palette near-miss copies of the mock (card `#2B251C` vs `#1A1714`, ink-low `#867C6D` vs `#6B6357`). **Why it matters**: every near-miss is a future bug; the mock's density (40px title band, 14px body padding) and the prototype's (30px band, 20px body offset) are different layouts wearing the same name. **Fix**: one table — tokens.rs becomes the single source; the sheet either uses the mock's exact values or declares the deltas in comments with reasons; DESIGN.md's stale scanline/radius entries get corrected to match tokens. Suggested command: `/impeccable audit` (technical quality) + `/impeccable polish`.
- **[P2] What**: Three worlds ship in one binary — warm ember (`--glass-b`), navy night-ops (default variants, `--island/--zone/--modal`), and the shipped `chrome.rs` look (rounded 12px islands, mono tracked-caps titles). **Why it matters**: there is no default answer to "what does tfg look like now"; reviewers compare the wrong pair of screenshots and the accent debate cannot close. **Fix**: declare the Tern direction the candidate product look, mark the p5/chrome passes as decision records in their headers, and make the entry flag state that. Suggested command: `/impeccable bolder` is wrong here; `/impeccable distill`.

## Persona Red Flags

**Alex (power user)**: Primary operator actions have zero accelerators — the only keys are lab keys (1-3, R, H, D, S, C). The Tern panels are not interactive at all (Painter draws with no `Sense`), so there is nothing to be fast at; the sheet's controls work but there is no path from "set a toggle" to "it took effect" beyond the STATE line. No undo, no batch anything. Alex concludes this is a demo, correctly.

**Sam (accessibility-dependent)**: The Tern pass is unreachable by keyboard and invisible to a screen reader — painted polygons, no focus, no semantics; the notch corner is admitted to be clickable-but-empty (`glass.rs:252-260`). Owning-input is communicated only as a painted rim color (blue), never as state text or focus; the resting rim at 34% alpha (`main.rs:788`) fails contrast against a moving map. The rust debug outline is permanent visual noise for low-vision users. The sheet is better (real widgets, real focus) but its focus ring is the accent color alone — no secondary indicator.

**Riley (stress tester)**: `--components` without `--glass-b` is silently ignored while the window resizes for it (`main.rs:1718` vs `:1212,:1754`) — a flag that lies. Captures via `--shoot` can fire mid-entrance; `shoot-egui.sh` sleeps 2s and calls that enough, but nothing enforces it. The "Body text at 15px." line renders at 13px — a promise the UI breaks on screen. Long hull names or a different locale would clip the fixed 420×250/380×250 panel rects with no scroll path.

## Minor Observations

- The picker pill's shadow is three stacked black rects standing in for blur (`main.rs:955-975`) while `glass.rs` uses a real blur — two craft answers for the same problem, 40 lines apart in spirit.
- `main.rs:788` hardcodes `#8C9BAE` (CUT_GREY's value) instead of `tokens::RESTING_RIM`; `glass.rs:161-164` hardcodes STEEL_LO/STEEL_HI values; `main.rs:1281` hardcodes the mock's ink-mid. Four literal re-declarations of values that already have names.
- The window title still says "p5-epaint — 1-3 / arrows switch · R replay · H hitbox" (`main.rs:1724`) — lab instructions as the product's title bar.
- `--glass-b --components` renders the sheet at 940×1000 over the map wash with no way to reach the frame pass except restarting with different flags; the two halves of the Tern answer never share a screen.
- `main.rs:1060-1111`'s bright-ground test palette (pale sea/land) is a third map palette alongside `glass.rs`'s wash and the app's real tiles.

## Questions to Consider

- If the mock's `#5B8DEF` is the committed accent, what becomes of `tokens::RADAR_CYAN` — and does ADR-0016 get a Tern variant ADR, or is the port a reversal of it?
- The mock spends "owns input" on the title label plus an outer glow; the prototype spends it on the rim plus the title. Which is the signal — and is the second spend deleted?
- Nothing in the Tern pass is a real console surface. Should the next pass port the top band and lifecycle row before any more frame work, or is the frame-first order deliberate?
- The binary currently contains three finished looks. Which one is the product, and do the other two get deleted, or kept as recorded decisions?
