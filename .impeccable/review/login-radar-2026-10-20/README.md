# The sign-in card, again: a name, a cursor, and one verb (2026-10-20)

Three changes to the sign-in card, and the reason for them is one thing: the
card was carrying a NAME, a CURSOR and a VERB, and it was carrying all three
as afterthoughts.

## What the pictures show

| capture | what it is |
| --- | --- |
| `signin-resting.png` | the card at rest: the Cinzel wordmark, the caret in the identifier field, the radar in its resting grey with the `SIGN IN` caption |
| `signin-resting-crop.png` | the same card, cropped to the card's own footprint |
| `signin-busy.png` | the round trip out: the radar LIT, the caption reading `SIGNING IN…`, the status line suppressed because the caption already says it |
| `signin-refused.png` | the server's answer: the radar back to grey, the fault stated once in red under the fields |

The pointer was over the app window for the resting capture, so the radar's
hover brightening is live in it.

## The three changes

**THE WORDMARK.** The name is the largest text on the console and the only
text allowed to be large, set in **Cinzel** — a Roman-capital inscriptional
serif, the letterforms of an engraved plate — tracked at a sixth of its size.
Cinzel is a third face, and the type system's rule is two faces and no
third; the rule stands for the console's TEXT, and this is the one place
where the text is a name rather than a reading.

**THE CARET.** The credential fields are hand-rolled — GPUI 0.2.2 ships no
text field, only the input plumbing — so the console draws the cursor too.
It stands after the value, in the focused field only, and it blinks on the
console's own tick, not the system's.

**THE SCOPE IS THE VERB.** The plain SIGN IN slab is gone; the radar is the
control, and the operator presses it. It brightens when the door is armed
and lights while the round trip is out, on two separate channels, because
"armed" and "waiting" are two facts and one colour spent on both is a
colour spent on neither.

## The capture harness, and the other agent

These pictures were taken AFTER the capture harness was fixed, and the fix
is the reason they show this console at all.

Two agents can run this console at once, each in its own git worktree. They
share the window CLASS (`tfg-gpui`), and the harness matched windows by
class first — so it picked whichever window the compositor listed first,
which may be the OTHER agent's. A capture then photographs somebody else's
console, and it looks exactly like your own edits having vanished.

The harness now matches on the PID it launched first, and kills only the
stale apps built from ITS OWN checkout, and writes its run log per checkout
rather than to one shared path. Two changes to `scripts/gpui-capture.sh`, and
one to the worktree's `.cargo/config.toml`: a worktree with its own
`target-dir` cannot have its binary overwritten by another checkout.

The proof that the fix works is in the binary: `strings target/debug/tfg`
carries `Cinzel` and does NOT carry `Command Center` — the string that
only exists in the other worktree's older sign-in card.
