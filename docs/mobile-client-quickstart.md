# A phone remote for TFG: build guide

This is the brief for building the Flutter client. It assumes you have the
repository checked out and can run the desktop console or `tfg-server` on the
machine that holds the map.

The phone talks **directly** to the TFG control endpoint over the LAN. It does
not SSH, does not run `tfgctl`, and does not talk to Minos. `tfgctl` is a
sibling reference client, useful for checking the server and for printing the
pairing QR code, but the app is its own client of the same HTTP/WebSocket API.

---

## 1. What the app does

The first version is a remote control, not a map renderer. That is a hard scope
line, and it comes from the server, not from taste:

- The API publishes a **camera** — latitude, longitude, zoom, viewport size —
  and accepts **semantic commands** to move it.
- It publishes **no tiles, no map image, and no game state**. `tfg-server` has
  no picture at all; the console has one, but there is no endpoint that streams
  it.

So the app should ship these four surfaces:

1. **Pair** — scan a QR code or type an eight-character code.
2. **Live camera readout** — latitude, longitude, zoom, and who moved it last.
3. **Motion** — drag to pan, pinch or buttons to zoom, and a "go to" form.
4. **Connection status** — paired as whom, connected/reconnecting, last event
   age, and a way to forget the device.

A map preview can come later, drawn by the app from its own tile provider, using
the camera in `camera.changed` as the viewport. Do not block the first version
on it.

### Words the API uses

| Term | Meaning |
| --- | --- |
| **host** | The process that owns the camera: `tfg` (the console) or `tfg-server` (headless). |
| **operator token** | The host's own long-lived credential. The phone never sees it. |
| **pairing code** | Eight characters, `XXXX-XXXX`, valid for 120 seconds, single use. |
| **device token** | 64 hex characters, minted when the phone spends the code. This is the phone's credential. Returned once. |
| **scope** | `observe`, `control_view`, `admin`. A phone needs the first two. `admin` is refused to paired devices on purpose. |

---

## 2. Before coding

### 2.1 On the host machine

The host binds loopback unless told otherwise. A phone on the LAN cannot reach
`127.0.0.1`, so bind all interfaces **and** set a token (a LAN bind refuses to
generate one, because that would print a credential to a terminal):

```bash
# headless host
cargo run --release --bin tfg-server -- \
  --listen 0.0.0.0:8787 \
  --token "$(openssl rand -hex 32)"
```

```bash
# or the console, hosting the camera you can see
TFG_CONTROL_LISTEN=0.0.0.0:8787 \
TFG_CONTROL_TOKEN="$(openssl rand -hex 32)" \
cargo run --release
```

Keep the printed token. It is the operator credential: it opens pairing windows
and revokes devices. It is not the phone's token and must not be baked into the
app.

Find the address the phone should use:

```bash
ip -4 addr show scope global | grep -oP 'inet \K[\d.]+'
```

That is usually the `192.168.x.x` or `10.x.x.x` address. Check it is reachable
from the phone's browser before writing any Dart:

```bash
curl http://192.168.1.20:8787/v1/hello
# {"protocol_version":1,"api_version":"v1","service":"tfg-control","revision":7,"pairing_open":false}
```

If that fails: the two devices are on different networks, the access point has
client isolation on (common on guest Wi-Fi), or a firewall is dropping 8787.
Fix it before writing code; it will not fix itself later.

### 2.2 On the phone

- Same Wi-Fi as the host, not the guest SSID.
- Android 9+ and iOS 14+ are the targets. Both block plain HTTP by default;
  section 6 has the two settings that unblock it.
- The app needs a QR scanner only for the fast path. Typing the code must
  always work, so build that first.

### 2.3 Flutter project

```bash
flutter create --org com.example --project-name tfg_remote tfg_remote
cd tfg_remote
flutter pub add http web_socket_channel flutter_secure_storage
flutter pub add mobile_scanner   # optional, scan path only
```

`web_socket_channel`'s `IOWebSocketChannel.connect` accepts headers, which is
required: the WebSocket needs `Authorization: Bearer <device token>` on the
upgrade request. Do not plan a web build — a browser cannot set that header.

---

## 3. The wire contract

Everything is under `/v1/`. JSON in, JSON out, UTF-8. Bodies are small.

| Method | Path | Auth | Purpose |
| --- | --- | --- | --- |
| `GET` | `/v1/hello` | none | Version and whether a pairing window is open. |
| `POST` | `/v1/pair` | none | Spend a pairing code, get a device token. |
| `GET` | `/v1/whoami` | bearer | Which grant this token is. |
| `GET` | `/v1/state` | bearer | Whole state, same shape as the WS snapshot. |
| `POST` | `/v1/commands` | bearer | Send one camera command. |
| `GET` | `/v1/ws` | bearer | WebSocket event stream. |

Errors always look like this, and `code` is the contract — branch on it, do not
parse `message`:

```json
{ "error": { "code": "forbidden", "message": "this credential may not send control_view commands" } }
```

The full contract is [`docs/control-api-v1.md`](control-api-v1.md). Sections 3–5
below are the subset the app needs, with exact field names.

### 3.1 The snapshot

Sent on connect to `/v1/ws`, and returned by `GET /v1/state`:

```json
{
  "type": "snapshot",
  "protocol_version": 1,
  "revision": 7,
  "views": {
    "main": {
      "id": "main",
      "camera": {
        "center": { "latitude": -6.108, "longitude": 106.91 },
        "zoom": 11.0,
        "width": 1280,
        "height": 720
      }
    }
  }
}
```

There is exactly one view today, `"main"`. Read it by key rather than by taking
the first entry, so a second display is additive rather than a crash.

### 3.2 The other events

```json
{ "type": "camera.changed", "revision": 8, "source": "phone-7",
  "command_id": "01J8Z6Q0K3",
  "view": { "id": "main", "camera": { "...": "..." } } }
```

```json
{ "type": "command.accepted", "command_id": "01J8Z6Q0K3", "revision": 8 }
{ "type": "command.rejected", "command_id": "01J8Z6Q0K3",
  "code": "invalid_command", "message": "latitude must be finite and between -90 and 90" }
```

- `source` is `"desktop"` for the operator's own hand, or a client id.
- `command.accepted` means the host took it. **`camera.changed` is the proof it
  moved.** Do not animate the camera off the HTTP 202.
- A client that falls more than 64 events behind receives a fresh `snapshot`
  instead of a patch. Treat any snapshot as authoritative and reset local state.
- You may send `{"type":"request.snapshot"}` on the socket to ask for one.
- Commands are **not** accepted over the socket. They go over HTTP.

### 3.3 Commands

`POST /v1/commands`:

```json
{
  "protocol_version": 1,
  "client_id": "phone-7",
  "command_id": "phone-7-42",
  "type": "camera.nudge",
  "payload": { "dx_fraction": 0.25, "dy_fraction": 0.0 }
}
```

Replies:

```json
{ "command_id": "phone-7-42", "status": "accepted", "revision": 8 }   // 202
{ "command_id": "phone-7-42", "status": "duplicate", "revision": 8 }  // 200, retry of a known id
```

The four command types:

| `type` | `payload` | Notes |
| --- | --- | --- |
| `camera.nudge` | `{"dx_fraction": f64, "dy_fraction": f64}` | Fractions of the host's viewport, each within ±2. Positive `dx` moves the camera **west**, positive `dy` **north** — the same sense as dragging the map. |
| `camera.zoom` | `{"delta": f64}` | Center-anchored, ±10 per command, host clamps to its zoom rails (3.0–18.0). |
| `camera.set` | `{"center": {"latitude": f64, "longitude": f64}, "zoom": f64}` | Absolute and idempotent. Latitude ±90, longitude ±180, zoom ±30. Out of range is a `422`, not a clamp. |
| `view.report` | `{"width": u32, "height": u32}` | Only for a client that **renders the map**. See the warning below. |

**`view.report` is the one command to be careful with.** A nudge is a fraction of
a viewport, so the host needs to know one. On the console, the window reports
its own frame; `view.report` is ignored. On `tfg-server`, the last report wins
for everybody. If the phone is a pure remote, **do not send it** — the host's
default geometry is fine, and reporting the phone's screen would resize every
other client's nudges. Send it only if the phone is the map display and nobody
else is.

Nudges are drag-like:

- finger moves right (`dx > 0`) → camera west → `dx_fraction = +dx / viewportWidth`
- finger moves down (`dy > 0`) → camera north → `dy_fraction = +dy / viewportHeight`

That sign is easy to get backwards. Check it against `tfgctl camera left`, which
sends exactly `{"dx_fraction": 0.25, "dy_fraction": 0.0}`.

`command_id` is optional, but send one. The host remembers recent ids for five
minutes; a retry with the same id answers `duplicate` and does not move the map
twice. Generate it from a per-install id and a counter, not a timestamp alone:

```dart
String nextCommandId() => '$_installId-${_counter++}';
```

---

## 4. Step 1 — pairing

### 4.1 The operator's side (this is the QR generator)

Pairing starts at the host, with the operator token. The reference client has
the whole flow in one command:

```bash
./target/debug/tfgctl \
  --server http://192.168.1.20:8787 \
  --token "$TFG_CONTROL_TOKEN" \
  pair --scopes observe,control_view
```

What that does:

1. `POST /v1/pairing` with those scopes, which opens a **120-second, single-use**
   window and replaces any window already open.
2. Prints the code, the payload, and — when stdout is a terminal — a scannable
   QR code.
3. Polls `GET /v1/devices` once a second and prints `paired dev-… phone` as soon
   as the phone spends the code, then exits.

Notes:

- `--scopes` defaults to `observe,control_view`, so you can omit it.
- It needs the **operator** token, not a device token.
- It cannot show the minted device token. That is returned only to the phone, in
  the `POST /v1/pair` response. The phone must make that call itself.
- Piped output omits the QR glyphs and stays parseable. `--json` gives one JSON
  object per line.
- If your terminal is light-on-dark, add `--no-color` (or set `NO_COLOR=1`) for
  a QR that scanners like better.

If you are testing without the CLI:

```bash
curl -sS -X POST http://192.168.1.20:8787/v1/pairing \
  -H "Authorization: Bearer $TFG_CONTROL_TOKEN" \
  -H 'content-type: application/json' \
  -d '{"scopes":["observe","control_view"],"ttl_seconds":120}'
```

The response is the offer:

```json
{
  "code": "K7QM-3XPD",
  "expires_in": 120,
  "uri": "tfg-control://pair?url=http://192.168.1.20:8787&code=K7QM-3XPD",
  "payload": "{\"protocol\":\"tfg-control\",\"version\":1,\"url\":\"http://192.168.1.20:8787\",\"pairing_code\":\"K7QM-3XPD\"}",
  "scopes": ["observe", "control_view"]
}
```

**The QR encodes `payload`**, not `uri`. `uri` is offered for a deep link. The
payload is the thing to parse:

```json
{ "protocol": "tfg-control", "version": 1,
  "url": "http://192.168.1.20:8787", "pairing_code": "K7QM-3XPD" }
```

Validate `protocol == "tfg-control"` and `version == 1`, and refuse anything else
with a readable message. The `url` is the host's guess at its own LAN address
(first non-loopback IPv4); on a machine with a VPN or Docker bridge it can be
wrong, so always let the user edit the URL and keep manual entry working.

### 4.2 The phone's side

`POST /v1/pair` is unauthenticated — the code *is* the credential:

```json
{ "code": "K7QM-3XPD", "label": "phone" }
```

```json
{
  "device": { "id": "dev-4b2379bfa714", "label": "phone",
              "scopes": ["observe", "control_view"],
              "created_at": 1793084168, "last_seen": 1793084168 },
  "token": "9f2c…",
  "url": "http://192.168.1.20:8787"
}
```

- Code matching ignores case, spaces and the dash, and folds `I`/`L` to `1` and
  `O` to `0`. Send what the user typed; no client-side normalisation needed.
- Wrong code → `401 invalid_pairing_code` while the window is still open. Five
  wrong guesses → `429 too_many_attempts`. Expired → `410 pairing_expired`.
  No window at all → `404 no_pairing_window`. Map these to four distinct
  messages; "pairing failed" is not one of them.
- The token is shown **once**. Store it in `flutter_secure_storage`
  (Keychain / Keystore), never in `SharedPreferences` and never in a log.
- Prefer the URL you connected to over the `url` in the response. The response's
  URL is the host's advertised address and may be the wrong interface.

`POST /v1/pair` refuses a token with `admin` no matter what the offer said, so
do not ask for it.

### 4.3 Dart skeleton

```dart
// lib/api.dart
import 'dart:convert';
import 'package:http/http.dart' as http;

class PairResult {
  PairResult(this.baseUrl, this.token, this.deviceId, this.scopes);
  final String baseUrl;
  final String token;
  final String deviceId;
  final List<String> scopes;
}

class ApiException implements Exception {
  ApiException(this.status, this.code, this.message);
  final int status;
  final String code;
  final String message;
  @override
  String toString() => 'ApiException($status $code): $message';
}

Future<PairResult> pair(String baseUrl, String code, String label) async {
  final response = await http
      .post(Uri.parse('$baseUrl/v1/pair'),
          headers: {'content-type': 'application/json'},
          body: jsonEncode({'code': code, 'label': label}))
      .timeout(const Duration(seconds: 8));

  final body = jsonDecode(response.body) as Map<String, dynamic>;
  if (response.statusCode != 200) {
    final error = body['error'] as Map<String, dynamic>? ?? const {};
    throw ApiException(response.statusCode,
        error['code'] as String? ?? 'unknown', error['message'] as String? ?? '');
  }
  final device = body['device'] as Map<String, dynamic>;
  return PairResult(
    baseUrl,
    body['token'] as String,
    device['id'] as String,
    (device['scopes'] as List).cast<String>(),
  );
}
```

Store the result:

```dart
final storage = const FlutterSecureStorage();
await storage.write(key: 'device_token', value: result.token);
await storage.write(key: 'base_url', value: result.baseUrl);
await storage.write(key: 'device_id', value: result.deviceId);
await storage.write(key: 'scopes', value: result.scopes.join(','));
```

---

## 5. Step 2 — live state and commands

### 5.1 The client

```dart
// lib/control_client.dart
import 'dart:async';
import 'dart:convert';
import 'package:http/http.dart' as http;
import 'package:web_socket_channel/io.dart';

class Camera {
  Camera(this.lat, this.lon, this.zoom, this.width, this.height);
  final double lat, lon, zoom;
  final int width, height;

  static Camera fromJson(Map<String, dynamic> c) {
    final center = c['center'] as Map<String, dynamic>;
    return Camera(
      (center['latitude'] as num).toDouble(),
      (center['longitude'] as num).toDouble(),
      (c['zoom'] as num).toDouble(),
      c['width'] as int,
      c['height'] as int,
    );
  }
}

class ControlClient {
  ControlClient(this.baseUrl, this.token, this.clientId);

  final String baseUrl;      // http://192.168.1.20:8787
  final String token;        // device token
  final String clientId;     // 'phone-<short install id>'

  IOWebSocketChannel? _socket;
  StreamSubscription? _subscription;
  Timer? _retry;
  Duration _backoff = const Duration(milliseconds: 250);
  int _counter = 0;
  int? _lastRevision;

  final _events = StreamController<Map<String, dynamic>>.broadcast();
  Stream<Map<String, dynamic>> get events => _events.stream;

  String _nextId() => '$clientId-${_counter++}';

  Map<String, String> get _auth => {'authorization': 'Bearer $token'};

  void connect() {
    _retry?.cancel();
    final wsUrl = baseUrl.replaceFirst('http://', 'ws://') + '/v1/ws';
    try {
      _socket = IOWebSocketChannel.connect(
        Uri.parse(wsUrl),
        headers: _auth,                       // required: the upgrade is authenticated
        connectTimeout: const Duration(seconds: 5),
      );
    } catch (error) {
      _scheduleReconnect();
      return;
    }
    _subscription = _socket!.stream.listen(
      (raw) {
        final event = jsonDecode(raw as String) as Map<String, dynamic>;
        if (event['type'] == 'snapshot') {
          _lastRevision = event['revision'] as int?;
        } else if (event['type'] == 'camera.changed') {
          _lastRevision = event['revision'] as int?;
        }
        _events.add(event);
        _backoff = const Duration(milliseconds: 250);   // a good message resets it
      },
      onDone: _scheduleReconnect,
      onError: (_) => _scheduleReconnect(),
      cancelOnError: true,
    );
  }

  void _scheduleReconnect() {
    _subscription?.cancel();
    _socket?.sink.close();
    _socket = null;
    _retry?.cancel();
    _retry = Timer(_backoff, connect);
    final next = _backoff * 2;
    _backoff = next > const Duration(seconds: 10) ? const Duration(seconds: 10) : next;
  }

  Future<void> send(String type, Map<String, dynamic> payload) async {
    final id = _nextId();
    final response = await http
        .post(Uri.parse('$baseUrl/v1/commands'),
            headers: {..._auth, 'content-type': 'application/json'},
            body: jsonEncode({
              'protocol_version': 1,
              'client_id': clientId,
              'command_id': id,
              'type': type,
              'payload': payload,
            }))
        .timeout(const Duration(seconds: 5));

    final body = jsonDecode(response.body) as Map<String, dynamic>;
    if (response.statusCode == 202 || response.statusCode == 200) return;
    final error = body['error'] as Map<String, dynamic>? ?? const {};
    throw Exception('${error['code']}: ${error['message']}');
  }

  // Drag deltas in logical pixels, viewport in logical pixels.
  Future<void> drag(double dx, double dy, double width, double height) =>
      send('camera.nudge', {
        'dx_fraction': dx / width,
        'dy_fraction': dy / height,
      });

  Future<void> zoom(double delta) => send('camera.zoom', {'delta': delta});

  Future<void> goTo(double lat, double lon, double zoom) => send('camera.set', {
        'center': {'latitude': lat, 'longitude': lon},
        'zoom': zoom,
      });

  void dispose() {
    _retry?.cancel();
    _subscription?.cancel();
    _socket?.sink.close();
    _events.close();
  }
}
```

Wire it up:

```dart
final client = ControlClient(baseUrl, token, 'phone-${installId.substring(0, 8)}');
client.connect();
client.events.listen((event) {
  switch (event['type']) {
    case 'snapshot':
      final views = event['views'] as Map<String, dynamic>;
      setState(() => camera = Camera.fromJson(
          (views['main'] as Map<String, dynamic>)['camera']));
    case 'camera.changed':
      final view = event['view'] as Map<String, dynamic>;
      setState(() => camera = Camera.fromJson(view['camera']));
    case 'command.rejected':
      showSnack('${event['code']}: ${event['message']}');
  }
});
```

### 5.2 Gesture details that matter

- **Do not send one command per pointer move.** A 60 Hz drag would be 60 HTTP
  requests a second. Accumulate the delta and send every 80–120 ms while the
  finger is down, plus one final command on release.
- **Nudge is relative; `camera.set` is absolute.** If the phone renders a map
  and you have a local camera, use `camera.set` after a gesture; if it does not,
  use `camera.nudge` and wait for `camera.changed` to update the readout.
- **Animate from events, not from optimistic state.** The host coalesces equal
  cameras and may clamp zoom. If you move the local camera first, reconcile
  against the next `camera.changed` instead of assuming your value won.
- **Disable controls while `scopes` lacks `control_view`.** Offer a read-only
  mode label instead of letting every tap become a `403`.
- **Ignore events with a `revision` lower than the last one you applied.** On
  reconnect, the snapshot resets the baseline.

### 5.3 Identity and revocation

`GET /v1/whoami` returns the grant the token resolved to. Call it once after
pairing and after every reconnect that returns `401`:

```json
{ "client": "dev-4b2379bfa714", "label": "phone", "scopes": ["observe", "control_view"] }
```

A revoked device gets `401 unauthorized` on the next request, including the
WebSocket upgrade. On that, delete the stored token, close the socket, and send
the user back to pairing with a message that says the device was revoked — not
"network error".

---

## 6. Platform configuration

### 6.1 Android

There is no TLS on this protocol yet. `https://` and `wss://` are refused by the
host, so the app must be allowed to speak plain HTTP to a LAN address.

`android/app/src/main/AndroidManifest.xml`:

```xml
<manifest ...>
  <uses-permission android:name="android.permission.INTERNET" />
  <application
      android:label="TFG Remote"
      android:usesCleartextTraffic="true"
      ...>
```

Flutter adds `INTERNET` to the debug manifest only. A release build needs it in
`main` as shown. `usesCleartextTraffic="true"` is broad; if that is unacceptable,
use a `network_security_config.xml` that permits cleartext for the LAN domain
instead. Either way, keep it out of any build that talks to the public internet.

Camera permission, only if you ship the scanner:

```xml
<uses-permission android:name="android.permission.CAMERA" />
```

### 6.2 iOS

`ios/Runner/Info.plist`:

```xml
<key>NSAppTransportSecurity</key>
<dict>
  <key>NSAllowsLocalNetworking</key>
  <true/>
</dict>
<key>NSLocalNetworkUsageDescription</key>
<string>Connects to the TFG map host on your local network.</string>
<key>NSCameraUsageDescription</key>
<string>Scans the pairing code shown by the TFG host.</string>
```

`NSAllowsLocalNetworking` covers LAN HTTP without opening the whole app to
arbitrary loads. iOS also shows a one-time "allow local network" prompt on first
connection; if the user declines, every request fails with a generic socket
error, so surface a hint that mentions Settings → Privacy → Local Network.

### 6.3 Desktop debug note

You can run the Flutter app on a desktop target for faster iteration, but the
final check must be on a phone: the cleartext settings, the local-network
permission and the QR scanner exist only there.

---

## 7. Test checklist

Run these in order. Each one has a specific failure to look for.

**On the host**

```bash
# 1. The endpoint answers without auth.
curl -sS http://192.168.1.20:8787/v1/hello

# 2. The operator token works and is the operator.
curl -sS http://192.168.1.20:8787/v1/whoami \
  -H "Authorization: Bearer $TFG_CONTROL_TOKEN"

# 3. A command moves the camera.
curl -sS -X POST http://192.168.1.20:8787/v1/commands \
  -H "Authorization: Bearer $TFG_CONTROL_TOKEN" \
  -H 'content-type: application/json' \
  -d '{"protocol_version":1,"client_id":"curl","command_id":"curl-1",
       "type":"camera.zoom","payload":{"delta":1.0}}'
```

**On the phone, in this order**

1. **Manual pairing.** Type the code from `tfgctl pair`. The app should receive
   `200` and a token. Verify with `GET /v1/whoami` that the scopes are the two
   you asked for.
2. **Reconnect.** Kill the app and reopen. It should use the stored token and be
   watching within a second or two, without pairing again.
3. **Live readout.** Move the map on the host with the mouse. The phone's
   latitude/longitude/zoom should follow within a tick. This proves the
   WebSocket, not just HTTP.
4. **Pan left.** Drag on the phone and watch the host's map. Dragging right must
   move the view west. If it is inverted, the fraction sign is wrong.
5. **Zoom.** Pinch or press zoom-in. The host clamps at 18.0; the readout must
   settle at the clamped value, not the one the app requested.
6. **Absolute go-to.** Set a latitude/longitude/zoom and confirm it lands there.
   Send it twice; the second must not move anything.
7. **QR path.** Open a new window, scan the QR, and confirm the payload parses
   and pairs. Then edit the URL field by hand and confirm repair works when the
   advertised address is wrong.
8. **Revocation.** From the host:
   ```bash
   ./target/debug/tfgctl --server http://192.168.1.20:8787 \
     --token "$TFG_CONTROL_TOKEN" devices
   ./target/debug/tfgctl --server http://192.168.1.20:8787 \
     --token "$TFG_CONTROL_TOKEN" revoke dev-…
   ```
   The phone's next action must land on the pairing screen with a "revoked"
   message, not a spinner.
9. **Wrong code.** Type a bad code five times. Expect four `invalid_pairing_code`
   and then `too_many_attempts`; the host must stop accepting guesses.
10. **Host restart.** Restart `tfg-server` with the same devices file. The phone
    reconnects on its own; the token still works. Restart with `--ephemeral` and
    the token is gone — that is expected, and the app should fall back to pair.
11. **Background/foreground.** Background the app for a minute, return. Either
    the socket survived or it reconnected; in both cases the readout must be
    current, not stale.

**What "working" means**

- The readout follows the host within a second.
- A drag moves the host's map in the direction of the finger.
- Killing and reopening the app does not require pairing again.
- Revocation is a clean return to pairing, not a hang.

---

## 8. Traps, collected

| Symptom | Cause | Fix |
| --- | --- | --- |
| `401` on every request | Token not sent, or `Bearer` missing. | Check the header, not just the token string. |
| `401` on the WebSocket only | `IOWebSocketChannel.connect` called without `headers`. | Pass `headers: {'authorization': 'Bearer $token'}`; browsers cannot do this, so no web build. |
| `403 forbidden` on commands | Token was paired `observe` only. | Re-pair with `observe,control_view`. Scopes are fixed at pairing time. |
| Connection refused from the phone | Host bound `127.0.0.1`. | Bind `0.0.0.0:8787`. |
| Timeout from the phone, works on the host | Guest Wi-Fi / AP isolation / firewall. | Same network, client isolation off, port open. |
| `SocketException: Cleartext HTTP traffic not permitted` | Android blocks plain HTTP. | `usesCleartextTraffic="true"`, or a scoped network security config. |
| iOS fails with a generic socket error | Local-network permission declined or ATS. | `NSAllowsLocalNetworking`, `NSLocalNetworkUsageDescription`, check Settings. |
| Nudges do nothing on `tfg-server` | No viewport has ever been reported, or a report was zero. | Do not send `view.report` unless the phone renders; the host has a default. |
| Nudge moves the wrong way | Sign flipped. | Positive `dx` is west, positive `dy` is north. |
| Zoom settles lower than requested | Host clamps to 3.0–18.0. | Believe `camera.changed`. |
| Duplicate moves on retry | New `command_id` per attempt. | Reuse the id when retrying the same intent. |
| QR scans but pairing fails | The QR's advertised `url` is the wrong interface (VPN/Docker). | Edit the URL, or type the code against the address that answered `/v1/hello`. |
| `tfgctl pair` never prints "paired" | The phone called the host but the CLI is pointed at another address or host. | `--server` must be the same host the phone reached. |
| App is asked for a password it does not have | The user pasted the operator token. | Paired devices get their own token; the operator token never goes in the app. |

---

## 9. What not to build

- **No SSH, no shell, no `tfgctl` invocation.** The app is an HTTP/WebSocket
  client. Anything that shells out to a CLI will not work on iOS and is not
  needed.
- **No raw gesture forwarding.** The protocol has no event or screen-coordinate
  message, and it should not grow one. Convert a gesture to a fractional nudge
  or an absolute camera.
- **No Minos credentials.** The phone never talks to the game backend. The host
  does, with credentials the phone must never see.
- **No `admin` scope.** Pairing refuses it. Revocation and re-pairing happen at
  the host.
- **No TLS assumptions.** There is no TLS yet. Do not ship an `https` default.

When the app is running against a host and you are unsure whether a failure is
the app or the host, `tfgctl` is the tiebreaker: if `tfgctl` can do it from
another machine on the same network, the API is fine and the app is wrong. If
`tfgctl` cannot, the host is not reachable and no amount of Dart will fix it.
