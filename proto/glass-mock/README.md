# tfg × Tern — three fusion directions

Throwaway design mock. Not product code. Delete freely.

Open it:

    python3 -m http.server 8740
    # then http://127.0.0.1:8740/index.html

Press `1`, `2`, `3` or use the switcher bottom-left to change variant.

## The three directions

| | Register | Where the game lives |
|---|---|---|
| A · Glass | Tern's register end to end | The map only. Reads as a web tool with a map in it |
| B · Instrument | Frame is the game | Notch, scanline band, steel bevel, corner brackets, one lit rim. Content inside stays glass |
| C · Full fusion | Both on every surface | Controls, cards, and fields all carry both registers |

Recommendation is B, recorded in the commit that added this dir.

## Accent

Signal blue `#5B8DEF`. Blue is already the friendly affiliation colour in
this repo (`src/main.rs:1017`), so every non-owned hull carries a neutral
ring instead of a second blue. The accent then means one thing: yours, live,
or the surface that owns input.

## Screenshots

    bash shoot.sh

Writes `shots/variant-{0,1,2}.png` at 1440×900. The mock must be served over
HTTP, because headless Chromium's `file://` loader rejects a query string and
`?v=` is how the mock picks a variant.
