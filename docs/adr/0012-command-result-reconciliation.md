# HelmOrder results reconcile only from authoritative answers

A HelmOrder result is explicit: `Draft`, `Pending`, `Accepted`, `Clamped`,
`Unknown`, `Refused`, or `Superseded`. The state describes the command's
truth, not the client's connection freshness.

## Rules

- A valid HTTP 201 `GameFixRecord` is the committed authoritative answer.
  The applied coordinates, heading, speed, scenario identity, and clamp fact
  come from that response; request values are never authoritative defaults.
- A malformed 2xx response, transport failure, or 5xx after submission is
  `Unknown`, not accepted or refused. The client does not automatically retry.
- Typed 4xx responses are `Refused` with an operator-facing category:
  not commander, wrong phase, actions closed, no applicable speed limit,
  stale game context, invalid request, or other refusal. The requested draft
  remains editable and resubmission is explicit.
- A matching, complete `game.order_issued` event may resolve an `Unknown`
  result when it carries the authoritative `GameFixRecord` identity. An
  incomplete or uncorrelatable event does not.
- A later disconnect does not downgrade `Accepted` or `Clamped`; it adds a
  separate freshness indication.
- A newer local intent may supersede an unconfirmed pending request. Pending
  commands are never queued.
- When MinOS clamps speed, the result is `Clamped`; the surface shows both
  requested and accepted speed, with accepted speed authoritative.

The API contract and known client seams are recorded in the research note
[`151-minos-heading-speed-order-contract.md`](../research/151-minos-heading-speed-order-contract.md).
