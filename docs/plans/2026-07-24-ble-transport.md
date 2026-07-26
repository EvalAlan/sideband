# BLE transport (bitchat-style offline messaging) — design & phased plan

**Decision (2026-07-24):** add a **BLE carrier** so Sideband works like bitchat
offline: walk into range of someone you know, no internet, **no OS Bluetooth
pairing**, and messages flow. The existing classic-RFCOMM carrier stays (it
works once addresses are known and is fine for larger payloads), but BLE becomes
the offline-first path.

## Why the current Bluetooth carrier can't do this

| | Classic RFCOMM (today) | BLE (this plan) |
|---|---|---|
| OS pairing/bonding | **required** | **none** |
| Finding a peer | must already know their address | **auto-discovery** |
| Address exchange | only via `transport_props` over Tor/LAN, or (new) the share code | not needed — discovery carries it |
| Reach | single hop | single hop now, **mesh relay** later |

Android 12+ also hides the local adapter MAC, so RFCOMM addressing falls back to
`name:<adapter name>` resolved against **bonded** devices — which is why pairing
is unavoidable there. BLE sidesteps all of it.

## The core design problem: recognizing a peer without pairing

A scanner sees BLE advertisements. It must answer "is this one of my contacts?"
without a prior address exchange, and without letting a third party track
anyone.

**Rotating advertisement IDs.** A device advertises

```
adv_id = SHA256( "sideband-ble-adv-v1" || account_pubkey || epoch )[..8]
epoch  = floor(unix_seconds / BLE_ADV_EPOCH_SECS)
```

A scanner recomputes this for **each known contact** over the current epoch ±1
(clock skew / rotation boundary) and matches. Properties:

- **Recognizable** by anyone who already has your pubkey (your contacts).
- **Not trackable** by anyone who doesn't: the ID rotates each epoch and reveals
  nothing about the key.
- **Cheap**: O(contacts × 3) hashes per advertisement, no crypto handshake to
  identify.

Matching a contact yields a BLE endpoint, which is stored like any other
transport property — so the existing route registry picks it up with no changes
to the send path's shape.

## Architecture (mirrors the existing Bluetooth bridge)

```
Rust core                          Android (Kotlin)
─────────                          ────────────────
transport/ble.rs   ←—Unix socket—→ BleBridge.kt
  • bridge protocol (JSON lines)     • BLE peripheral: advertise adv_id
  • adv-id derive + match            • BLE central: scan + match + connect
  • frames in/out (BTP)              • GATT service: write + notify chars
                                     • MTU fragmentation/reassembly
```

Rust stays the protocol owner: the Kotlin side only moves **opaque frames** and
reports discovery, exactly like `BluetoothBridge.kt` does for RFCOMM.

**Small MTU:** BLE gives ~185–500 bytes per write. The **BTP-micro codec**
already written for LoRa (parked on `bridges-parked`) targets exactly this and
is directly reusable for fragmentation.

## Phases

Each phase is TDD where it can be, leaves the tree green, and keeps every
existing carrier working.

### BLE-1 — advertisement identity (pure Rust, testable now)  ← START
- `ble_adv_id(account_pubkey_b64, epoch)`, `ble_current_epoch(now_ms)`.
- `match_ble_adv_id(contacts, adv_id, now_ms)` → the matching contact, checking
  epoch ±1.
- Tests: a contact's own advert matches; a stranger's doesn't; the id rotates
  across epochs; matching tolerates a one-epoch skew.

### BLE-2 — Rust bridge protocol (`src/transport/ble.rs`)
- Commands to the platform: `start_advertise{adv_id}`, `start_scan`,
  `connect{peer}`, `write{session, frame}`, `disconnect`.
- Events from the platform: `discovered{peer, adv_id, rssi}`, `connected`,
  `frame{session, bytes}`, `disconnected`, `error`.
- Same Unix-socket + JSON-lines pattern as `transport/bluetooth.rs`, with the
  same framing/timeout hardening. Unit-test the codec + state machine.

### BLE-3 — wire into discovery + routing
- On `discovered` → match to a contact → store a `ble` transport property
  (auto-populated; no out-of-band exchange).
- `RouteEndpoint::Ble` in the registry, priority between LAN (100) and classic
  BT (80) — BLE is likelier to be usable than RFCOMM since it needs no bonding.
- Send path unchanged in shape: BLE just becomes another candidate route.

### BLE-4 — Kotlin `BleBridge.kt` (needs the user's phones to validate)
- Peripheral: `BluetoothLeAdvertiser` advertising the service UUID + adv_id in
  service data; GATT server with a write characteristic + a notify characteristic.
- Central: `BluetoothLeScanner` filtered on the service UUID; connect; MTU
  negotiate; write/notify.
- Fragmentation via BTP-micro; reassembly with per-session buffers + timeouts.
- Permissions: `BLUETOOTH_ADVERTISE`, `BLUETOOTH_SCAN` (+ `BLUETOOTH_CONNECT`),
  and the `neverForLocation` scan flag so no location permission is needed.

### BLE-5 — surface + clients
- Settings toggle ("Offline Bluetooth (BLE)"), status in the UI, foreground
  service wiring so scanning/advertising survives backgrounding.
- Rebuild APK; iterate with real two-phone testing.

### BLE-6 — mesh relay (optional, the full bitchat behaviour)
- Multi-hop forwarding with a TTL/hop limit and message-id dedup (the
  `sync_seen_items` primitive already exists), plus store-and-forward for peers
  briefly out of range.

## Non-negotiables
- BLE is a **carrier only**: messages stay Sideband-E2E (Ed25519 + X25519 +
  Double Ratchet). Unlike bitchat, peers must still be **contacts** — one
  in-person QR scan — because the crypto is identity-based. That's the
  intentional trade: no anonymity-by-default, but no key-exchange-by-proximity
  either.
- Advertisements must not expose a stable public identifier (hence rotation).
- Every existing carrier (Tor / LAN / RFCOMM) keeps working unchanged; BLE is
  additive and opt-in.

## Honest constraints
The radio half (BLE-4/5) **cannot be verified in this environment** — it needs
two real phones. Rust-side logic (BLE-1/2/3) is unit-testable and will be
tested; the Kotlin side will be build-verified and then iterated against real
hardware testing.
