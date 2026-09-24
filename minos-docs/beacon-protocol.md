# Position beacon protocol — for the Flutter client

Written 2026-09-15. Audience: the Flutter team building the on-board app.

**Status: the reporting policy below is agreed, and the ingest endpoint IS BUILT** —
`POST /api/v1/live-feed/fixes`, authentication only (the service checks that the caller is that
vessel's current commander). The request shape in §6 is what it accepts. Anything that changes will
be reflected here.

---

## 1. What the app is

A **beacon**. It reports where its vessel is, and nothing else.

- It has **no read access** to anything. It does not display positions, does not know where other
  vessels are, and never needs to.
- One signed-in account is one Commando, commanding one vessel. The app is an attribute of that
  assignment, not a separate identity — so it uses the ordinary login and needs no device credential.
- The **Rust desktop** is the command centre and displays every vessel. It is a different client with
  a different contract.

## 2. Why not once per second

The obvious design is "send a fix every second". It is not affordable, and the reason is arithmetic
rather than preference:

| | 1 vessel | 5 000 vessels |
|---|---|---|
| Fixes per day at 1 Hz | 86 400 | **432 million** |
| Fixes per 30 days | 2.6 million | **13 billion** |
| Storage per 30 days | — | **~1.3 TB** |

And a 30-knot vessel moves about **15 metres** between fixes. On any chart the command centre draws,
that is smaller than a pixel. So a fixed 1 Hz costs a fleet-scale database to move ships by an
invisible amount, and it drains the device battery doing it.

**This problem is already solved.** Ships have broadcast their positions over shared radio for
decades, under the AIS standard (ITU-R M.1371), and it faced the same constraint — a channel only so
big, shared by everyone. Its answer was to make the reporting rate follow how fast the situation is
actually changing. We are adopting the same intervals, which means a navy audience recognises the
behaviour instead of being surprised by it.

## 3. The rule

**Report every `I` seconds, where `I` depends on speed and on whether the vessel is turning.**

| Vessel state | Interval `I` |
|---|---|
| Moored or anchored, and not moving faster than 3 kn | **3 min** |
| Moored or anchored, but moving faster than 3 kn | 10 s |
| 0–14 kn | 10 s |
| 0–14 kn **and turning** | **3⅓ s** |
| 14–23 kn | 6 s |
| 14–23 kn **and turning** | 2 s |
| Faster than 23 kn | 2 s |
| Faster than 23 kn **and turning** | 2 s |

"Turning" is a *state*, not a single sample — a noisy course reading must not toggle it:

> Enter the turning state when a fix differs in course from the last reported fix by **more than 5°**.
> Leave it **60 seconds** after the last such change.

**2 seconds is the floor.** Nothing reports faster than that under any circumstances.

Two things this table is doing that are worth knowing:

- **The moored row is also the liveness signal.** Do NOT add "skip the report when nothing has
  changed" as an optimisation. If a stationary vessel stops reporting entirely, the command centre
  cannot tell it apart from an app that crashed or a phone with a dead battery — which is a fact the
  exercise depends on knowing. One fix every three minutes already cuts the volume by 99.4% and keeps
  that distinction intact.
- **The turning rows exist for the display.** A manoeuvring vessel is exactly where sparse reporting
  is most visible, so the rate rises where the error would. This is why the desktop can draw a smooth
  track from relatively few fixes.

For a typical patrol pattern — say 20 hours moored, 4 hours at 12 knots with some manoeuvring — this
is roughly **2% of the 1 Hz traffic**, with no loss of anything the command centre can display.

## 4. Reading course and speed — do not compute them

**The GPS receiver already provides them.** Do not derive them from consecutive positions.

- Android: `Location.getSpeed()` (m/s), `Location.getBearing()` (degrees).
- NMEA underneath: `$GPVTG`, `$GPRMC`.

Send the receiver's values. They are a measurement, not a reconstruction, and the command centre
needs them to draw between fixes.

**Two fields must be nullable, and null is correct data, not an error:**

- **Course is meaningless when the vessel is nearly stationary.** Below roughly 0.1 kn the bearing
  spins on jitter — a moored ship with a swinging stern. Send `null`, not a made-up bearing. AIS does
  the same thing, and a spinning icon alongside is a worse lie than no course at all.
- **Speed** may be unavailable from a cold receiver, or indoors.

## 5. Send in batches, not one request per fix

Buffer fixes and flush:

- every **15 seconds**, if the buffer has anything in it;
- immediately when the buffer reaches **60 fixes**;
- immediately on **reconnect**;
- on **app lifecycle events** (backgrounded, stopped), so a fix in hand is not lost.

One request per fix at fleet scale is thousands of requests per second against the API. Batching
turns that into hundreds, and it is the same mechanism the offline case below needs anyway.

## 6. Request shape

> **The response code below is out of date — see the note under it.** The endpoint itself is built
> and accepts exactly the shape shown.

```
POST /api/v1/live-feed/fixes
Authorization: Bearer <token>

{
  "device_id": "a stable id for this install",
  "fixes": [
    {
      "recorded_at": "2026-09-15T20:14:02Z",
      "latitude":    -6.1234567,
      "longitude":   106.1234567,
      "speed_kn":    12.4,
      "course_deg":  87.5,
      "accuracy_m":  8.0
    }
  ]
}
```

- `recorded_at` — **when the fix was taken**, from the device clock. Required.
- `latitude` / `longitude` — required, WGS84 decimal degrees.
- `speed_kn`, `course_deg`, `accuracy_m` — nullable. Omit or send `null`.
- `device_id` — a stable identifier for the installation, so a device swap is visible in the record.
  It grants nothing; it is for audit.

**Maximum 60 fixes per request, and no fix older than 30 days.**

Response:

```
202 Accepted
{ "accepted": 7, "rejected": 0 }
```

> **DISCREPANCY — the server answers `200 OK`, not `202 Accepted`.** This document was written before
> the endpoint existed, and the two have never been reconciled. The body above is right.
>
> **200 is the more accurate of the two, and the reason is the body itself.** `202` means *"accepted
> for processing; it has not finished"* — but this response reports `accepted` and `rejected` counts,
> which cannot be known until processing HAS finished. A `202` carrying a completed result contradicts
> itself. The ingest is synchronous: every fix is stored or refused before the response is written.
>
> So the correct fix is to change this line to `200 OK`, not the server. **Confirm before you build
> against it** — if the Flutter app already checks for `202`, the server changes instead, and that is
> a one-line change on our side.

## 7. Offline — buffer, then backfill

Reporting gaps are expected: coverage, battery, aeroplane mode, the app being killed.

- **Buffer fixes while offline and send them on reconnect, keeping the ORIGINAL `recorded_at`.**
- **Measure the clock offset on reconnect** and shift the backlog by it. Consumer device clocks drift
  by roughly ±20 ppm — about 72 ms per hour — so a single offset applied backwards is sound against
  intervals measured in seconds.
- **Never drop the backlog.** A gap in the track and a silent vessel are different facts, and the
  command centre reports them differently. Dropping fixes destroys the distinction.

We timestamp **every fix with the server's arrival time as well** (`received_at`). Live reporting is
not affected by a wrong device clock, and a `recorded_at` in the future is flagged as a diagnostic
rather than silently corrected. You do not need to solve clock accuracy — just send what you have and
tell us the offset for the backlog.

## 8. Device realities to plan for

- **Android:** continuous reporting with the screen off requires a **foreground service** with the
  `location` service type and a persistent notification. Background location permission is separate
  from foreground, and is a review-visible permission. Start this early — it is the part that fails
  late.
- **Do not point the device clock at a network time source.** Stock Android and iOS cannot be pointed
  at a custom NTP server, and nothing depends on it — see §7.
- **Request GPS at a steady rate and apply the interval table in the app.** Do not try to express the
  table through `setIntervalMillis`; the OS treats requested intervals as a hint, not a guarantee, and
  the policy above lives on your side.
- **Battery:** this policy is one of the biggest favours this app does itself. At 2 s while turning
  and 3 min alongside, the radio and the GPS spend most of their time idle, and the backend traffic is
  ~2% of the naive design.

## 9. Please do not

- **Do not send at 1 Hz "to be safe".** It is ~50× the traffic and shows nothing extra.
- **Do not derive course or speed from previous positions.** The receiver has them.
- **Do not send a made-up course for a stationary vessel.** Send `null`.
- **Do not drop the offline backlog**, and do not re-stamp backfilled fixes with the current time —
  the original time is the whole point.
- **Do not add "skip when unchanged".** See §3: it breaks liveness detection.

## 10. How we will check it

Acceptance for the beacon, so it is not a matter of opinion:

| Scenario | Expected |
|---|---|
| Moored, still, 1 hour | ~20 fixes (3-minute interval) |
| 12 kn, straight, 1 hour | ~360 fixes (10-second interval) |
| 15 kn, manoeuvring, 1 hour | ~1 800 fixes (2–6 second intervals) |
| Offline 30 minutes, then reconnect | every fix arrives, with its original `recorded_at` |
| Screen off, app backgrounded, 1 hour | reporting continues |

If a run reports 3 600 fixes per hour for a moored vessel, the interval table is not being applied.
