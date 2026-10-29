# ADR-0027: The session's access token is kept alive by one rotation thread

A console that ran longer than an hour stopped working, and the only cure was
to sign in again. Minos issues an hour-long access token beside a refresh token
that rotates on every use; the console kept the access token and never rotated
the chain, so the wire was refused with 109 and the plot read answered 401
(`game feed: plot read failed: unauthorized`) while an exercise was still
running. The refresh path existed (`MinosAuth::refresh`, the keyring store, the
`RefreshDue` event) but nothing called it.

## Status

Accepted. Builds on the auth grill's resolution (login, rotation, keyring) and
carries no change to the socket, the shell or the Minos contract.

## Context

Three properties of the credential decide the design, and each one is a trap if
it is treated as an implementation detail:

1. **The access token is short-lived and the session is not.** A TNI AL exercise
   runs for hours. A token that is only replaced at login is a token that
   expires mid-exercise.
2. **Rotation is destructive.** `/auth/refresh` answers with a new refresh token
   and invalidates the one presented; a replay is rejected as theft. Two
   rotations racing each other therefore leave one caller — and possibly the
   keyring — holding a burned token.
3. **The keyring entry is the next launch's only credential.** A rotation that
   is not written back means the next launch rotates a token the server has
   already burned, and the operator gets the card again: the exact complaint
   this ADR answers.

Two clocks also disagree: `expires_in` is the server's claim, and the local
monotonic clock stops while the machine is suspended. So a clock-only schedule
is right almost always and wrong exactly when it matters (a laptop that slept
through the expiry).

## Decision

**One keeper (`src/backend/session.rs`), one rotation thread, and a push.**

- **Single-flight is structural.** Exactly one thread calls `/auth/refresh`.
  Everything else — the socket's 109, a 401 on a plot read — asks it to
  (`TokenKeeper::nudge`). No mutex guards a shared call, so a race cannot
  produce two rotations and there is no "who wins" to reason about.
- **Two triggers, one schedule.** The proactive one is the clock: rotate a
  slack before `expires_in` (a tenth of the token's life, capped at five
  minutes). The reactive one is a refusal, and it wins, because the server is
  the authority on what a token is worth. Both land on the same `due` instant.
  A reactive attempt is floored at thirty seconds so a socket retrying a
  burned token cannot rotate the chain in a loop.
- **The keeper owns the refresh chain and its keyring entry.** It writes every
  rotated refresh token under the identifier it was installed with. The
  environment's capture credential installs with no identifier and stays in
  memory: a capture's account is not the machine's own.
- **A refused refresh ends the session.** 401 from `/auth/refresh` is not
  retried — the chain is dead and a retry is a replay. The keeper clears the
  token and the keyring entry and reports `TokenEvent::Expired`; the binary
  performs the same teardown a sign-out does (hold cell, live wire). The card
  comes back on the next launch. A transient failure (transport, 5xx) keeps the
  token and retries at thirty seconds, then a minute: a network blip does not
  sign an operator out of a running exercise.
- **Rotation is pushed, not polled.** The keeper knows no socket and no shell.
  It emits `TokenEvent::Rotated` on a channel and `main.rs` has one follower
  thread that writes the hold cell (the plot reader's token) and sends
  `LiveCmd::SetToken` (the socket's connect data). The write path and the shell
  read `TokenKeeper::current()` per command, so they never hold a copy that can
  go stale. `sync_session` takes its token from the keeper for the same reason.

## Consequences

- **A console left open through an exercise keeps working.** The plot reader
  never sees more than one 401 for a given token, and the socket's next
  handshake carries the rotated token.
- **The shell is not told.** A dead chain leaves the console running and
  refusing writes with its existing "signed out — sign in to …" sentences; the
  card returns on the next launch. Surfacing "your session ended" mid-window
  needs a binary → shell feed that does not exist yet, and inventing one inside
  the GPUI migration would have meant a fourth channel into the same window.
- **The keepalive and the wire are one credential.** `hold.token` is written by
  the follower thread and by `sync_session`; both take the value from the
  keeper, so the two cannot disagree about which token is the session's.
- **Rotation costs one extra round trip per expiry.** The proactive rotation
  lands five minutes before the old token dies; the socket handshaked with the
  old one, so its 109 at the hour mark asks for a rotation again. That second
  rotation is harmless (the chain is the server's, not the client's) and it is
  cheaper than teaching the actor to report which token its connection carried.
- **A one-second token cannot loop.** Below a minute the slack is zero and the
  rotation fires at expiry; a token that short is a server defect the client
  reports, not one it amplifies.

## Verified

- `backend/session.rs` is tested against stub servers: a rotation replaces the
  token, reports it, and the second rotation presents the token the first one
  minted (the chain is honoured); a 401 ends the session and stops (no replay);
  a transport failure keeps the token and reports nothing; a signed-out keeper
  rotates nothing; and the slack curve (3600 → 300 s, 1000 → 100 s, 600 → 60 s,
  59 → 0) is asserted rather than assumed.
- The end-to-end claim — a console outliving its access token — needs the real
  API and a clock, so it is the operator's check: leave the console open past
  the hour and confirm no 109 and no `plot read failed` line.
