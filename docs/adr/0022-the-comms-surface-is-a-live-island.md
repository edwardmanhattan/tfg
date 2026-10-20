# ADR-0022: The comms surface is a live island, and a tab per traffic type

The console's comms panel was a photograph of the last write. A message
that arrived on the socket was logged and thrown away; an ADDRESSED
message could not arrive at all; the composer could send exactly one
shape of message; and a failed inbox read drew as an empty inbox. The
panel was also a centred, dimmed modal — which the console's own comment
on the field said it should never be, because "an inbox is something you
read beside the map, and a dim over the map is the one thing that makes
it unreadable."

## Status

Accepted.

## Context

Minos's messaging contract was read from the server's own source and
then probed against the deployed API (2026-10-21). What it says:

- **Two kinds, and only two.** `telegram` (between the personnel) and
  `administrative` (between the Judges and the Game Master). A request
  with a third value is refused: *"This field must be one of telegram
  administrative."*
- **Three markings.** `TERBUKA` (open), `TERBATAS` (restricted) and
  `RAHASIA` (classified), each with a border colour the SERVER owns
  (`#FF0000` for RAHASIA, `#000000` otherwise). Only `TERBUKA` may be a
  broadcast — *"A message with no `to` and no `cc` is a broadcast, and
  only `TERBUKA` may be a broadcast."*
- **An audience, or none.** `to` and `cc` are lists of USER ids, and a
  recipient must be a participant of the exercise. An empty audience IS
  the broadcast; there is no flag for it.
- **A degree**, a foreign key into the server's own catalogue
  (`BIASA`, `SEGERA`, `SANGAT SEGERA`, `KILAT`), read from `/helpers`.
- **An assumed identity.** A judge's telegram must be sent AS a scenario
  role this game issued, and must name the node it speaks FROM. An
  administrative message needs neither.
- **A receipt**, per recipient, on ADDRESSED messages only.
- **Two deletions.** The author's withdrawal is gone for everybody; the
  recipient's hide is gone for that recipient alone — and the server
  answers the second with **404** even though the tombstone is written.
- **Reads** have no phase gate but do require the caller to be a
  participant; **sends** require the exercise to be in `execution`.
- **The realtime channels.** A broadcast is published on `game:<id>`; an
  addressed message is published on `personal:<user id>` and on NOTHING
  else — not even a TERBUKA one.

What the console did with that contract:

1. It watched only the game's channel, so **no addressed message could
   ever arrive**. The command to watch the personal channel existed in
   the client and was never sent.
2. Even a broadcast was **logged and dropped** (`main.rs`: `eprintln!`)
   and never reached the surface. The inbox was whatever the last
   session read had found.
3. The projection kept six of the server's fields. The marking's CODE
   was thrown away, the degree with it, the receipt with it, the reply
   link with it, and the server's own border colour with it — so the
   difference between a routine telegram and a classified one was
   invisible.
4. The composer could send one thing: a `telegram`, `TERBUKA`, degree
   1, broadcast, with no audience and no identity.
5. The inbox was read at `page_size=100` and the count on the surface
   was the LENGTH of that page, so a busier exercise's count was a lie
   about the exercise.
6. A failed read — a 403 for an account that is not on the roster, or a
   dead server — was swallowed into `unwrap_or_default()` and drew as
   *"No messages in this exercise."*
7. The panel was a centred modal behind a 150-alpha black sheet, with
   no scroll, no close control, and no way out of it in execution.

## Decision

**The inbox is live, the message is whole, the surface is an island, and
the tab strip is the server's own vocabulary.**

### The live path

- The socket watches the **personal channel** as well as the game's, and
  the watch is declared with the ACCOUNT, not with the session: an
  addressed message may arrive for an exercise the console is not holding
  right now.
- The server has already subscribed every account to its own personal
  channel, so the explicit subscribe is answered **105 "already
  subscribed"** — which the stock SDK reads as a refusal and answers by
  dropping the channel. The vendored SDK now takes 105 as the
  subscription it names: the channel IS subscribed, and the
  publications are the whole point of subscribing.
- A live arrival reaches the shell on **its own feed**, not as another
  arm of the session answer: nothing was asked for, there is no re-read
  to apply, and the operator's own draft must survive it.
- The arrival is **merged by id**, never appended blind. The next
  re-read brings the same message again, and a list that doubles a
  message is a list nobody trusts.

### The message is whole

Every field the server publishes is read and carried: the marking's code
and label, the degree, the receipt, the reply link, the assumed role,
the author id where the caller is entitled to it, and the recipient list
with each recipient's own receipt.

The server's `border_color` is **parsed and deliberately not used**. The
console's palette is owned by `src/view/mod.rs`, and a server hex would
bypass it — and every pixel assertion with it. The marking's CODE is
mapped to a console ink instead: TERBUKA to the hairline's grey,
TERBATAS to `DATA_AMBER`, RAHASIA to `FAULT_RED`.

Two facts the server does not publish per-caller are computed by the
client, because the server publishes them per-recipient and the console
knows which recipient it is: whether THIS caller was addressed
(`to_me`) and when THIS caller read it (`read_at`).

### The composer sends the whole envelope

The composer offers kind, marking, degree, audience, the identity a
judge speaks as, and the message it answers. The shell **refuses
locally what the server would refuse** — a non-TERBUKA broadcast,
administrative mail from a commander's seat, a judge's telegram with no
role, a reply on a telegram, more than 8000 characters — because the
console's law is to hide a verb the backend would refuse rather than
showing one that 409s. The server still has the last word, and its
refusal is the sentence the surface shows.

### The read is the server's read

One read, at the server's maximum page, with the server's own count
beside it, so a truncated inbox can say so. A failed read is a SENTENCE
on the surface, never an empty list.

### The surface is an island, and the tab strip is the vocabulary

The surface is a floating island over the map — opaque, square, 1px
`CUT_GREY` edge, undimmed — with its own close control, a scrolling
list, and no full-frame sheet over the map.

The kind strip is **ALL / TELEGRAM / ADMIN**, one tab per kind the
server has, each carrying its count. A second, smaller scope row
(**ALL / TO ME / BROADCAST**) narrows the tab. The tabs are drawn in the
console's existing idiom (the assessment strip: small verbless targets,
`PANEL_SLATE` fill and `MAP_INK` when picked) because GPUI 0.2.2 ships
no tab widget and the console has never drawn one.

The marking is a **per-row treatment, not a tab**: the operator asks
"telegram or administrative?", not "open or secret?" — the latter is a
property of a single row.

## Consequences

- The console can now be *told* things: a message that arrives while
  the operator is looking at the map reaches the surface, is marked
  NEW, and is announced in its type's own ink and tone.
- The two kinds look different, and the difference is the difference the
  server makes: a telegram is radio traffic between the personnel and
  carries the console's live accent (`RADAR_CYAN`); administrative mail
  is staff correspondence and carries `WARNING_SAND` — attention without
  fault. A classified message carries `FAULT_RED` on its row.
- The console says what it does not know: a message sent under an
  assumed role shows the assumed identity and no author, because the
  server withholds the author there — from everybody, the Game Master
  included.
- The receipt matrix is now possible: the sender can see who has read an
  addressed message, and a broadcast has no receipts to show because the
  server refuses them.
- The console is no longer able to send a message the server will
  refuse, unless the refusal depends on something only the server knows
  (a recipient who commands nothing, a hull that is not in the task
  organisation) — and those refusals arrive with the server's own
  wording.

### Limitations, stated plainly

- **The audience picker cannot tell a commander from a non-commander.**
  The roster does not say who holds a command post, and a telegram
  addressed to a seat that holds none is refused by the server. The
  console offers the roster and lets the server rule; it never greys a
  seat out on a guess.
- **`id_type` is not offered.** The column is a foreign key to a
  `message_types` table with no rows and no route, so every value the
  server accepts on the wire is refused by the key.
- **A busy exercise still pages.** The session read asks for the
  server's maximum page (200); the surface says how many of how many it
  is holding.
- **The publish path is the console's own.** A message reaches a
  participant by being SENT: the console does not publish a message to
  a participant's socket.

## Verified

Against the deployed development API, 2026-10-21:

- A broadcast telegram sent by another account **arrived live** on the
  console's socket.
- An ADDRESSED administrative message — which the console could not
  receive at all before — **arrived live** on the personal channel, and
  the console logged both.

And the crash that hid all of it: the current-step parse indexed the
server's answer, and an ABSENT key panics. A step whose window was never
set has no `window_end_at` at all, and the session read died on it
before the window opened. Every field of that answer is now read
through `get`: a server answer is data, and data that does not fit the
shape is missing, never fatal.
