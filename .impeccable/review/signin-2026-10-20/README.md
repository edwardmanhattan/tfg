# The sign-in, overdriven (2026-10-20)

The sign-in surface was the one place in the console still wearing the old
world — a rounded card inset 24pt on two sides, floating on a flat void. It
is now a modal of this console, centred on the window, over a theatre.

**The direction:** *The Theatre + The Scope*.

- **The theatre.** The console's own live basemap, full-window, under a flat
  `CONSOLE_NIGHT` scrim at 0.86 — no gradient, no blur, no glass. The map
  thread runs from the launch, so the picture is already on screen when the
  operator arrives: the sign-in happens over the water the exercise will be
  fought on. The scrim lifts from opaque night to the ghost over 0.9s on an
  exponential ease-out, tied to the PICTURE rather than to a timer, and it
  only ever lifts.
- **The door.** A modal, centred on the window: two credential fields
  (username, password), the verb, and a status line. The fields are the
  console's form order — tracked-caps label over a well, the value in the
  MACHINE face, an empty field carrying the console's dash rather than a
  hint word. The submit is a square graphite slab that brightens to
  `MAP_INK` when both credentials are typed: readiness in ink, not in the
  accent, since the one cyan edge on this surface is the field that owns the
  keyboard.
- **The scope.** Under the form, in the room the pre-flight readouts used to
  take: the console's own compass face — rim, two rings, twelve graduations,
  a north wedge, a needle — drawn in greys. There is no figure on it and
  nothing to read off it. A chart room's door has a scope on it, and the
  water the exercise will be fought on is right there under the card.

## What was taken out, and why

The surface used to carry a **PRE-FLIGHT**: a UTC clock, whether a picture
had landed, the camera's zoom and bearing, and the frame's own size. Four
real readings — and a diagnostics panel, which is what it was. An operator
who has to type a username does not need a stopwatch and a frame size
before the door opens.

The clock also cost the surface its own repaint: the map renders on a
gesture, so a clock driven by the map's frames stops the moment the theatre
settles, and the sign-in had to ask for frames on the second boundary to
keep it ticking. With no clock, the drain is gone too.

The status slot is RESERVED rather than conditional: a card that grows a
line when the server answers moves the scope under the operator's hands, and
decoration that moves is a bug in the decoration.

## The pictures

| file | what it shows |
|---|---|
| `signin-resting.png` | the theatre arrived, nothing typed |
| `signin-credentials-ready.png` | both fields filled; the submit has woken |
| `signin-busy.png` | the card mid-flight (`TFG_SIGNIN=busy`) |
| `signin-refused.png` | the server refused (`TFG_SIGNIN=refused`) |
| `signin-signed-in-console.png` | the console AFTER the sign-in, over the hosted API |

`TFG_SIGNIN=busy|refused` stages the card's states for a headless capture,
the same seed-the-state precedent as `TFG_SETTINGS`.

## How they were verified

**The scrim is a number, not a look.** The same sea pixel, measured in both
surfaces: the signed-out theatre reads `srgb(48,56,71)`, the signed-in
console reads `srgb(245,243,238)`. That is exactly 0.86 of the console's
night over the daylight basemap's white sea — `THEATRE_SCRIM = 219` — so the
scrim is where the code says it is.

**The arrival only ever lifts.** `the_theatre_arrives_from_night_and_settles_on_the_ghost`
asserts the alpha starts at 255, settles on 219, stays there, and never
rises again on the way down. It is tied to the PICTURE, not to a timer, so
the theatre never fades into a map that is not there.

**The signed-in console was re-shot after the rewrite.** The first pass
could not photograph it: the hosted API did not answer, every launch fell
back to the fixture roster, and the console never left the card. That was
transient. With the API up, `scripts/gpui-sweep.sh` re-shot the five
phases and the three modals with the pixel assertions attached, and the
signed-in column's edges, the owned field's cyan perimeter and the
one-rule-between-fields all still hold.

**`cargo test --lib`: 619 pass**, the same four pre-existing failures.

## What is still owed

- The basemap under the theatre is still the DAYLIGHT `liberty` style the
  visual-integrity audit named as a product answer rather than a patch. The
  scrim makes it a night map at 0.86, which is a surface answer; the
  audit's answer is still owed.
- The theatre is the basemap at the console's DEFAULT camera, because there
  is no session before the sign-in and so nothing to aim at. An operator
  who signs in somewhere else finds the theatre was Jakarta all along.
  Whether the sign-in should aim at the last session's ground is a product
  answer.
- Whether the bottom instrument row survives contact with a 1040x640 window,
  which is the app's hardcoded launch size. Measured, not assumed.
- `PRODUCT.md`'s `## Platform` is `desktop`, which the Impeccable schema does
  not recognise (it warns, and treats the project as web). The field carries
  a note explaining the deliberate out-of-enum choice; it was left alone
  rather than repaired as a side effect of a drawing.
