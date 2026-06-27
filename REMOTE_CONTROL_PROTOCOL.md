# Remote Control Protocol

Sideband's `serve` command can expose a TCP control port for remote GUI clients
(especially Android, where `libsideband.so` is unavailable).

## Usage

```bash
sideband serve --profile ~/.sideband --remote-addr 0.0.0.0:9999
```

The port speaks a **JSON-line protocol**: one JSON object per newline, same as the
stdin/stdout control channel used by the desktop GUI.

## Protocol

### Client → Server (Commands)

Each line is a `ServeControlCommand` JSON object:

```json
{"cmd": "send", "to": "contact_name", "message": "hello"}
{"cmd": "group_send", "group": "group_id", "message": "hello"}
{"cmd": "file", "to": "contact_name", "path": "/path/to/file"}
{"cmd": "group_leave", "group": "group_id"}
{"cmd": "group_delete", "group": "group_id"}
{"cmd": "retry_status"}
```

### Server → Client (Responses)

Each line is a `ServeResponse` JSON object, tagged with `type`:

```json
{"type":"ack","cmd":"send"}
{"type":"sent","cmd":"send","to":"contact_name"}
{"type":"error","cmd":"send","kind":"resolve","message":"unknown contact"}
{"type":"group_sent","cmd":"group_send","group":"group_title","sent":3,"total":3}
{"type":"file_sent","cmd":"file","to":"contact_name"}
{"type":"left","cmd":"group_leave","group":"group_title"}
{"type":"deleted","cmd":"group_delete","group":"group_title"}
{"type":"retry_status","queued":0}
```

Responses are broadcast to **all** connected clients, so every client sees
every response. Clients should filter by `cmd` or `type` as needed.

## Android Integration

When `libsideband.so` fails to load on Android, the Dart code should:

1. Connect to the remote control port (e.g. `10.0.0.15:9999` over Tailscale)
2. Send `{"cmd":"send","to":"...","message":"..."}` for outgoing messages
3. Listen for `{"type":"sent",...}` or `{"type":"error",...}` responses
4. For incoming messages, the serve instance handles them via Tor and the
   response is broadcast to all connected clients

### Dart Pseudocode

```dart
// Fallback when libsideband.so is unavailable
final socket = await Socket.connect(remoteHost, remotePort);
final reader = socket.transform(utf8.decoder).transform(LineTransformer());

// Send a message
socket.add(jsonEncode({'cmd': 'send', 'to': contact, 'message': msg}));
socket.add('\n');

// Listen for responses
reader.listen((line) {
  final resp = jsonDecode(line);
  switch (resp['type']) {
    case 'sent': // mark message as sent
    case 'error': // show error
    case 'ack': // message accepted for sending
  }
});
```

## Hermes Bridge

When `--hermes-bridge` is enabled, the serve instance intercepts inbound
messages starting with the prefix (default `!`), pipes them to `hermes chat -q`,
and sends the response back via Sideband. This works identically for remote
clients — the bridge is on the serve side, not the client side.

## Security

- Bind to `127.0.0.1` for local-only access
- Bind to `0.0.0.0` only behind a firewall/VPN (Tailscale recommended)
- No authentication on the control port — anyone who can connect can send/receive
- All Tor encryption/decryption happens on the serve side
