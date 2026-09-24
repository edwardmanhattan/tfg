# Real-time protocol — for the Rust command centre

**Status: BUILT and verified end to end.** A beacon batch posted to `POST /live-feed/fixes` reaches a
subscribed socket client, through the full deployed chain. Everything below was checked against the
running system rather than derived from the code.

This document is the contract for the command centre. It is the mirror of `beacon-protocol.md`, which
is the contract for the Flutter handset: one describes how positions go IN, this describes how they
come OUT.

---

## 0. Read this first — there is no official Rust SDK

Centrifugal Labs ships official client SDKs for the **browser** (JavaScript, Dart), **mobile**
(Swift, Java, Dart, C#) and **backend** (Go, Python). **Rust is not among them.** The Centrifugo client
protocol is open and documented, so this is not a blocker, but it does mean you are not calling a
blessed library and this is worth knowing before you plan the work.

What exists:

- **[`tokio-centrifuge`](https://crates.io/crates/tokio-centrifuge)** — an *unofficial* Rust client
  SDK. As of September 2026 it is at **0.2.6**, which is pre-1.0: expect the API to move. **Pin the
  exact version** and read its changelog before upgrading. It is the realistic path.
- **`tauri-plugin-centrifugo`** — if the command centre is a Tauri app, this wraps the above and
  saves you the integration.

**SSE / HTTP-streaming were looked at as a way to avoid the SDK question, and they are NOT a
shortcut.** Recorded here so nobody has to re-open it:

- SSE is **DISABLED** in this deployment. `/connection/sse` returns 404 while `/connection/websocket`
  responds, so the endpoint is not registered at all. It is off by default and would need
  `sse: { "enabled": true }` in the Centrifugo configuration.
- SSE is **unidirectional by nature**, so Centrifugo pairs it with a **second endpoint** —
  `/emulation` — that the client POSTs its commands to. Connecting and subscribing therefore becomes
  two moving parts instead of one. It is not a plain "HTTP GET that never ends".
- Centrifugo's documentation states this transport **"is only implemented by our JavaScript SDK at
  this point"**. It does not remove the need for a Rust implementation; it adds one on top.

So **`tokio-centrifuge` over WebSocket is the shortest path**, unofficial as it is. We are happy to
enable SSE if you find a use for it, but it is not a way out of §0.

---

## 1. What this is

Centrifugo is a standalone pub/sub server that sits in front of our API. Your client opens **one**
connection to it and multiplexes every subscription over that connection. Our API never handles the
socket; it only *publishes* into Centrifugo, and Centrifugo *asks* our API whether a connection or a
subscription is allowed.

```
   your client  ──WebSocket──►  Centrifugo  ──gRPC──►  Minos API
                                                    (who are you? may you watch this?)
        ▲                             │
        └─────── publications ────────┘
                                      ▲
                              Minos API (on ingest)
```

Two consequences that are easy to get wrong:

- The socket is **not on the API's port**. Centrifugo listens on **`:8000`**; the API is on `:8080`.
  `:9090` is a gRPC port between Centrifugo and the API — client code never touches it.
- **REST and real-time are separate paths to the same data.** Polling `GET /live-feed/positions`
  still works and is not deprecated. Use it for the initial picture and the socket for updates.

---

## 2. Connecting — the token goes in `data`, not a header

```
ws://<host>:8000/connection/websocket
```

The connection token is sent in the **connect frame's `data`** field, as JSON:

```json
{ "token": "<access_token>" }
```

**Not** an `Authorization` header, and this is not a style preference — a browser WebSocket cannot set
headers at all, so Centrifugo forwards whatever the SDK puts in the connect frame. Use the same field
regardless of your transport.

`<access_token>` is the **same access token** `POST /auth/login` already returns. There is no separate
real-time token and nothing extra to fetch.

### What the server answers

On success Centrifugo replies with:

| field | meaning |
|---|---|
| `user` | your account id, as a string. This is your identity for the life of the connection. |
| `expire_at` | when this connection ends — **the token's own expiry**, see §8. |
| `channels` | **`["personal:<your id>"]`** — already subscribed for you, server-side. |

> **Do not subscribe to `personal:<your id>` yourself.** The server has already done it, so an explicit
> subscribe returns **105 already subscribed**. That is not an error; it is the feature working.

---

## 3. Signing in first

`POST /auth/login` (see the OpenAPI spec). Note the field is **`identifier`**, not `username`.

Two cases are refused at connect, both deliberately:

- An account with **`must_change_password`** set. The HTTP layer refuses such an account everywhere, and
  the socket would otherwise be a side door around that gate. Change the password first.
- A token with **no expiry**. A token with no stated lifetime would authorise a connection that never
  ends, so it is refused rather than treated as eternal.

---

## 4. The channels

| channel | who may subscribe | what it carries |
|---|---|---|
| `live-feed` | **every authenticated caller** | every vessel's newest position — see §5 |
| `personal:<user id>` | that account only | messages addressed to one user ("you now command this hull") |
| `game:<id>` | only if `GET /games/{id}` would be readable to you | exercise lifecycle, roster, readiness |
| `game:<id>:positions` | same check as `game:<id>` | per-exercise position stream |

> **These two are authorised but NOTHING EMITS YET.** The channels work and the authorisation works;
> no event has been implemented. The payloads below are the agreed contract so that neither side has
> to guess — see §4a. Tell us which ones your command centre actually needs and we will implement
> them in that order rather than all at once.

### 4a. `game:<id>` — the exercise event stream

**Status: specification, not yet emitting.** Defined before implementation so the shapes are agreed.

#### The rule that decides every payload here: a publication is a BROADCAST

`game:<id>` is subscribable by **participants**, not only staff — the authorisation is the same check
`GET /games/{id}` performs, and participants pass it. And a Centrifugo publication is delivered to
**every** subscriber of the channel, with no way to address one of them.

Therefore **a payload on `game:<id>` must be safe for the least privileged subscriber**, because there
is no mechanism that could make it otherwise. That is the constraint the whole section hangs on.

Concretely: `area` and `map_tag` are withheld from participants until the exercise is in execution
(concept [2.2] — a Commando does not know the ground until it is Executed). So:

- **No `game:<id>` event carries `area` or `map_tag` while `GameHasStarted()` is false.**
- `game.state_changed` is the **one** event that carries them, and only when the new state is execution
  or closure — which is exactly the moment the withholding ends. The event that reveals the ground is
  the event that starts play.

Anything a Game Master must see but a Commando must not **cannot go on this channel at all**. It needs
a staff-only channel, which we have not built.

**Messaging — the first feature to hit this — solved it the other way round.** An addressed message is
never published here: it goes to each recipient's `personal:<id>` channel instead, so a `RAHASIA`
message never reaches a channel every participant subscribes to. That is why `personal.message_sent`
exists as a type of its own rather than as a second audience for `game.message_sent`. A staff-only
channel is still unbuilt, and messaging did not need one.

#### The envelope

Every event on this channel, whatever its type:

```json
{
  "type": "game.state_changed",
  "id_game": 12,
  "occurred_at": "2026-09-15T10:20:00Z",
  "data": { }
}
```

| field | notes |
|---|---|
| `type` | the discriminator. **Switch on it and ignore types you do not know** — new ones will be added without ceremony, and a client that fails on an unknown type breaks on every server release. |
| `id_game` | matches the channel, so a client multiplexing several exercises can route without tracking which subscription fired. |
| `occurred_at` | server clock, when the change committed. |
| `data` | type-specific, below. |

#### The events

| `type` | emitted when | `data` |
|---|---|---|
| `game.state_changed` | a transition commits | `{ "from": "preparation", "to": "execution" }`, plus `"area"` and `"map_tag"` **only when the new state has started** |
| `game.roster_changed` | a participant is added, removed, or has their role changed | `{ "action": "added"\|"removed"\|"role_changed", "id_user": 9, "name": "…", "role": "…" }` |
| `game.readiness_changed` | a participant declares or withdraws readiness | `{ "id_user": 9, "name": "…", "ready": true, "declared_at": "…" }` |
| `game.units_changed` | a hull is assigned to, released from, or re-commanded within the exercise | `{ "action": "assigned"\|"released"\|"commander_set", "id_unit": 13, "name": "KRI Ahmad Yani", "hull_number": "KRI-AH-YN" }` |
| `game.organisation_changed` | the task organisation moves — a hierarchy node is created, edited, deleted, reparented, or a unit is placed or unplaced | `{ "action": "…", "id_node": 7, "id_unit": 13 }` |
| `game.placement_changed` | a hull's **starting position on the map** is set or cleared — before play begins only | `{ "action": "set"\|"cleared", "id_unit": 13, "latitude": -6.0, "longitude": 106.0 }` |
| `game.clock_changed` | the scenario clock is paused, resumed, or its rate changed | `{ "action": "paused"\|"resumed"\|"rate_changed", "running": false, "time_factor": 6, "assumed_now": "…" }` |
| `game.order_issued` | a Commando's order is applied: the current leg closes and the next opens | `{ "id_unit": 13, "assumed_time": "…", "latitude": -6.0445, "longitude": 106.0001, "heading_deg": 180, "speed_kn": 20, "requested_speed_kn": 35, "clamped": true }` |

Every one of these mirrors an existing HTTP operation, so there is no new authority here: **an event
carries exactly what the corresponding read would have shown the same caller.**

#### `game.message_sent` is a BROADCAST and nothing else

Published when a message is sent with **no `to` and no `cc`** — which A2 defines as visible to every
game participant, making this channel exactly its audience. `data` is the message, shaped exactly like
`GET /games/{id}/messages/{message_id}` returns it:

| `type` | emitted when | `data` |
|---|---|---|
| `game.message_sent` | a message is sent to the whole exercise | the `GameMessage` of the API contract, with `broadcast: true` |

**AN ADDRESSED MESSAGE NEVER APPEARS HERE**, not even a `TERBUKA` one: "addressed" is what the
recipient list means, and publishing it here would disclose it to everyone. It goes to
`personal.message_sent` on each recipient's personal channel instead.

#### `personal.message_sent` — the one event that is not on a game channel

An addressed message is published to each recipient's `personal:<id>` channel, once per recipient.
`personal:<id>` is granted by the connect proxy to exactly the user it names, so the channel **is** the
recipient list and no filter is needed on the client.

| `type` | emitted when | `data` |
|---|---|---|
| `personal.message_sent` | a message names this user in `to` or `cc` | the same `GameMessage` shape |

**A DIFFERENT TYPE RATHER THAN THE SAME ONE ON A DIFFERENT CHANNEL.** A notification banner needs to
tell "this is for me" from "this is for the exercise", and the channel alone cannot say it once a
client is multiplexing both.

**THE EVENT NEVER CARRIES THE REAL AUTHOR WHEN AN IDENTITY WAS ASSUMED.** `sender.assumed_role` is
present and `sender.author` is absent — **for every subscriber, the Game Master included**. A published
event has no caller to tailor it to: the HTTP read can show a Judge who really sent a message because
it knows who is asking, but an event is written once and read by whoever is subscribed. Staff read the
author over HTTP when they need it, which is what F5 asks for anyway.

When **no** identity was assumed there is nothing to conceal, so `sender.author` is present exactly as
it is over HTTP — otherwise a client could not attribute a broadcast at all.

#### `game.order_issued` is the one that draws a moving plot

Worth stating because it is not obvious, and it is the reason this channel does not need a position
event per second: **a position is a closed-form function of the leg in force and the elapsed assumed
time.** So an order event carrying the new leg is *sufficient* for a client to advance that hull
itself, forever, with no further events — the position formula is in `README.md` §2 and is the same
one the server uses.

That is what makes `game:<id>:positions` a **convenience rather than a requirement**: it exists so a
client can be dumb about it, not because the data is otherwise unavailable. Two consequences for you:

- **`clamped` and `requested_speed_kn` are on the event on purpose.** An order for 35 kn applied at
  30 is a thing the Commando must be told, and the payload is deliberately the same shape the ORDER
  response returns so the two can be reconciled without a lookup.
- **`assumed_time` is on the SCENARIO clock**, like every other instant here. It is the stamp of the
  leg, not the wall clock time the order arrived.

#### `game:<id>:positions`

Same payload as `live-feed` (§5), filtered to the hulls assigned to that exercise. Separate from
`game:<id>` because the two have different rates and different consumers: a Game Master watching the
roster should not have a fix per second per hull down the same socket, and a client that wants only the
map should not have to receive roster events to get it.

`FeedPositionEvent` carries no `area` or `map_tag`, so it is inherently participant-safe and the §4a
rule does not bite here.

**Deferred at the client's request — 2026-09-21. Re-checked 2026-09-22: still silent, and the
interim answer is now a real read.** The Rust team is not working on this channel yet, so it stays
specified and nothing publishes to it. Nothing on our side blocks it: the channel is already
authorised and only the publish is missing.

> ⚠️ **The interim answer is `GET /games/{id}/positions`, and it is NOT `/live-feed/positions`.**
>
> An earlier version of this note said "the command centre polls the HTTP read for positions", which
> was ambiguous when it was written and is now wrong in a way that matters, because there are two
> reads and they are different things:
>
> | read | what it returns | who may call it |
> |---|---|---|
> | `GET /live-feed/positions` | the live feed — positions **measured** by reporting devices aboard real vessels, in no game | authentication |
> | `GET /games/{id}/positions` | an exercise's **derived** plot, advanced along the fix chain to an assumed instant | a participant of that game, or staff |
>
> The command centre's plot wants the second, and the first will not draw a game at all: a hull
> ordered to steam along a rhumb line is not reporting from a device. The two are kept apart
> deliberately, because a measurement is never clamped and a derived position is.

`GET /games/{id}/positions` is **built** (2026-09-21). Two parameters to know before building
against the poll:

- **`?at=<RFC3339>`** — an instant on the **scenario** clock, not the wall clock. Omitted, it means
  the scenario `now`. A client that sent its own clock would be asking about a different day, which
  is the easiest mistake to make here and the only one this read can be wrong about.
- **`?from_unit=<id>`** — turns the read into a measure. Every position then also carries
  `distance_m`, `bearing_deg` (great-circle, and **not symmetric** — the bearing back is not this
  plus 180) and `relative_bearing_deg` from the observer's own live heading, in `[-180, 180)`.

**A participant sees the whole plot** — every hull, including the other side's and the judge side's.
Fog of war is a future release. That is deliberately the opposite of `GET /games/{id}/units`, which
stays staff-facing so a Commando does not learn the order of battle *before* the exercise; the two do
not contradict each other, because the plot does not exist until play begins. See `README.md` §1.1.

Polling this at exercise scale is fine, and it is a decision rather than an oversight. When the
channel lands, the payload above is the agreed shape.

#### Reconnecting — you will have missed events

There is **no message history** configured on these channels, so a reconnect does not replay what was
missed. Events are low-frequency, so the simple answer is the right one: **re-read `GET /games/{id}`
(and the roster) after every reconnect, then resubscribe.** Treat the socket as an accelerator over the
read, never as the only source of state. That is the same discipline as §9.

If you would rather have true gap-free recovery, Centrifugo supports channel history with offset-based
recovery, and we would enable it on the `game` namespace. Ask — it costs us config and gives you
replay, but the re-read above is enough for events this slow.

The authorisation rule for a game channel is the **same check the HTTP read uses** — not a parallel
one. A subscriber list even slightly wider than the reader list would leak through the socket exactly
what `GET /games/{id}` withholds.

Channel names are built by the server and handed to you. **Do not construct them yourself**: a
malformed name is refused (107) precisely because it means a client is inventing names rather than
being told them.

---

## 5. `live-feed` — the position event

One JSON object per publication. **One message per beacon batch, per vessel** — not one per fix.

```json
{
  "id_unit": 13,
  "name": "KRI Ahmad Yani",
  "hull_number": "KRI-AH-YN",
  "latitude": -6.0888,
  "longitude": 106.9111,
  "speed_kn": 14.2,
  "course_deg": 87.5,
  "accuracy_m": 4.5,
  "recorded_at": "2026-09-15T09:56:28.923518653Z",
  "received_at": "2026-09-15T09:56:28.923518653Z",
  "backfilled": false
}
```

### The fields

| field | notes |
|---|---|
| `id_unit` | join key against `GET /live-feed/positions` |
| `name`, `hull_number` | the unit's label, so a marker is drawable without a prior lookup |
| `latitude`, `longitude` | always present, always finite |
| `speed_kn`, `course_deg`, `accuracy_m` | **optional — the key is ABSENT when unmeasured** |
| `recorded_at` | when the fix was TAKEN |
| `received_at` | when the server ACCEPTED it |
| `backfilled` | **read §6 before you draw anything** |

**The optional three are omitted, never zero.** A receiver that has not acquired a fix reports none of
them, and a course below roughly 0.1 kn is jitter rather than a measurement. A `0` sent here would be
indistinguishable from a measured zero and drawn with exactly the same confidence — so if the key is
missing, render nothing. Do not default it to 0.

### There is no `age_seconds`

The HTTP read carries one; this event deliberately does not. Age is measured against the moment of
**rendering**, so a value frozen at publish time is wrong the instant it arrives and grows more wrong
the longer you stay connected. Compute it yourself from `received_at` — a server that sent it would be
sending an under-estimate dressed as a measurement.

---

## 6. `backfilled` — do not skip this one

A handset that loses signal buffers its fixes and flushes them when it reconnects. That batch can be
**days** old. When it lands, the vessel's newest fix is a point from hours or days ago.

`backfilled: true` means: *this is real, it is the newest thing we know, and it did NOT just happen.*

- `backfilled: false` — the fix is live. `recorded_at` and `received_at` will be effectively identical,
  because the server re-stamps live fixes with its own clock. Move the marker.
- `backfilled: true` — reconstructed from the offline buffer. The vessel **was** there; it may not be
  there now. Draw it on the track, and **do not** treat it as the vessel's current position.

Verified both ways against the running system:

| fix age | `recorded_at` vs `received_at` | `backfilled` |
|---|---|---|
| live | identical | `false` |
| ten minutes late | 10 minutes apart | `true` |

**If you ignore this field, every vessel that drives through a dead zone will teleport across your map
when it comes back.** That is the single most likely way to get this wrong.

---

## 7. Refusals, and what to do about each

These are Centrifugo's client-protocol codes, not HTTP ones, and the number tells your client whether
to retry. Handle them explicitly rather than treating every failure as "reconnect".

| code | meaning | what to do |
|---|---|---|
| `101` | unauthorized — bad, absent or unverifiable token | **Do not retry.** Sign in again. |
| `103` | permission denied | **Do not retry.** You may not watch that channel. |
| `105` | already subscribed | Not an error. Ignore, or do not double-subscribe. |
| `107` | invalid channel name | **Do not retry.** A name was built client-side; use the server's. |
| `109` | token expired | **Retry after refreshing the token** — see §8. |
| `100` | internal error | **Retry**, with backoff. Our side failed, not yours; it is marked temporary. |

The `101` / `109` split matters: **101** means do not come back, **109** means refresh and come back. A
client that treats them the same either retries forever with a token that will never work, or signs its
officers out every hour.

`100` is worth honouring too. If our database blips, you get `100` rather than a refusal, precisely so
that a blip does not look permanent — a client that gives up on `100` never sees the data after the
blip ends.

---

## 8. Reconnection and token expiry

The connection **cannot outlive the token that opened it**. `expire_at` in the connect reply is the
token's own expiry, and when it lapses Centrifugo disconnects you.

So the lifecycle is: **refresh the access token, reconnect, resubscribe.** This costs one reconnect per
token lifetime and needs no refresh proxy on our side. Plan for it rather than being surprised by a
periodic disconnect — it is the design, not a fault.

**Reconnect on:** transport failure, `109`, and `100`.
**Do not reconnect on:** `101`, `103`, `107` — these will fail identically forever.

A reconnect must be **exponential backoff with jitter**. Do not spin: a client that hot-loops on a
refused connection is a retry storm, and it will look like our outage.

---

## 9. The socket and the read must agree

Initial state comes from `GET /live-feed/positions`; updates come from the socket. Use **both**:

1. `GET /live-feed/positions` for the whole picture, including vessels that have **never** reported
   (they appear with no `position` — that is deliberate, a vessel silently missing from a list is
   indistinguishable from one that does not exist).
2. Subscribe to `live-feed` and apply events on top.

Subscribe **before** or concurrently with the initial read, and reconcile by `id_unit` — otherwise a
fix published between the read and the subscribe is lost with no way to notice.

`backfilled` is derived by the same code on both paths, so the read and the socket cannot disagree
about whether the same fix is live. If you ever see them disagree, that is a bug on our side; tell us.

---

## 10. How to check it

Ask us for the dev command centre address and a test account, and:

1. `POST /auth/login` → take `data.access_token`.
2. Connect to `ws://<host>:8000/connection/websocket` with `{"token": "<token>"}` in `data`.
3. Subscribe to `live-feed`.
4. **We will POST a fix for a test vessel**, or you can post one yourself with a commander account via
   `POST /live-feed/fixes`.
5. You should receive a publication matching §5, with `backfilled: false`.

Nobody has to guess: the API logs every publish it makes, and the system has already been verified end
to end with a real beacon payload.
