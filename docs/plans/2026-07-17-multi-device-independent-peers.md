# Multi-device (independent peers) — design & phased plan

**Decision (2026-07-17):** Sideband will support **one account across multiple
devices**, using the **independent-peers** model (Signal-style): every device is
standalone and works when any subset is online. The alternative (hub +
companions, phone authoritative) was considered and rejected because the user
wants phone-can-be-off independence.

This is a multi-session **epic**, not a feature. Each phase is TDD, leaves the
tree green, and keeps a **single-device account working unchanged** (a
single-device account is just a device list of length 1).

## Why this is hard (the two blockers)

Grounded in the current architecture, where **identity and reachability are
decoupled**:

- **Crypto identity** = the account's Ed25519 signing key + X25519 key in
  `identity.toml`. Contacts trust you by your `pubkey_b64`.
- **Reachability** = a Tor onion whose key lives in Arti's keystore under
  `<profile>/arti_state`. It is *not* derived from the identity key. Contacts
  store your `.onion` to reach you.

You therefore **cannot** just copy a profile onto a second device:

1. **Tor descriptor races** — two live devices sharing one onion key both
   publish the same hidden-service descriptor; last-writer-wins, so delivery
   becomes unreliable. (Tor solves this only with OnionBalance-style infra.)
2. **Double Ratchet desync** — the ratchet is *pairwise and stateful*. Two of
   your devices sharing one ratchet session with a contact each advance it
   independently → permanent desync + a security break. So **every device needs
   its own ratchet session per contact**, which forces the sender to know
   *which* device a message is for → a device list + fan-out.

## The model

Your **account** = an Ed25519 **account identity key (AIK)** (the existing
`identity.toml` key) + a **set of devices**. Each device has:

- its own **device Ed25519 key** (on the primary, the device key *is* the AIK —
  device 0),
- its own **onion** (its own `arti_state`),
- its own **X25519** key and its own **ratchet sessions**.

The AIK signs a **device certificate** per device
(`account_pubkey · device_pubkey · onion · x25519 · added_at · caps`), and the
collection is a **signed device list** (versioned, list-level signature prevents
silently dropping a device or replaying an old version).

**Trust chain:** contact trusts your AIK → AIK vouches for each device cert →
each message is signed by a device key that must appear in your current signed
device list. Contacts store your *device list* (not a single onion).

**Sending** to a contact = **fan-out** to each of their devices, each over its
own `(contact, device)` ratchet session.

**Receiving:** contacts fan-out to all your devices, so each device gets its own
copy directly.

**Self-sync:** your own devices run an encrypted device-to-device channel to
mirror state that isn't delivered by fan-out — sent-message echoes, read
receipts, contact-list changes, device-list changes — queued through the
existing retry path when a device is offline, deduped via `sync_seen_items`.

## Open decision (resolve at Phase 3)

**Who can authorize a new device?** Default plan: **primary-only** — only the
device holding the AIK secret can sign new device certs (smallest blast radius),
with an **AIK backup inside the encrypted export archive** as the recovery path
if the primary is lost. Alternative: copy the AIK to every device (any device
can link, but one compromise exposes the account root). Revisit when Phase 3
lands.

## Phases

Each phase = TDD, green tree, single-device still works.

### Phase 1 — Account/device core (pure crypto + storage)  ← STARTED
- `DeviceCert` / `DeviceList` types (serde) + canonical, domain-separated
  signing bytes.
- `sign_device_cert` / `verify_device_cert`, `sign_device_list` /
  `verify_device_list(expected_account_pubkey)`.
- `build_self_device_list(profile, onion)` (primary; size-1, device 0 = AIK) +
  persist/load to `<profile>/devicelist.toml`, **encrypted at rest**
  (`read/write_encryptable`).
- Tests: cert sign/verify round-trip; tamper (onion/x25519) rejected; wrong
  expected-account rejected; dropped/replayed device in a list rejected;
  injected non-account cert rejected; build+load round-trips and verifies.
- No behavior change; nothing wired into send/receive yet.

### Phase 2 — Contact = many device-endpoints  ← 2a + 2c DONE
- **[done] 2a** — a contact's devices live in an encrypted per-contact sidecar
  (`<profile>/contact-devices/<name>.toml`), separate from `ContactFile` so
  legacy single-onion contacts need no format change. `contact_endpoints()`
  returns the verified list's endpoints or a single legacy endpoint (device 0
  == the contact's account key). Commit `58bd329`.
- **[done] 2c-core** — per-`(contact, device)` ratchet keying
  (`RatchetState::path_for_device`, device 0 keeps the legacy path),
  `build_outbound_message_for_endpoint`, `build_outbound_messages_fanout`;
  interop test proves a 2-device contact yields two correctly-encrypted
  messages. Commit `a4e5a5a`.
- **[done] 2c-send** — `send_in_conversation` fans a message out to a contact's
  *extra* devices best-effort (device 0 still drives status/history/retry;
  extras are fire-and-forget, skipped on retries). No-op for single-device
  contacts. Commit `362e873`.
- **[moved to Phase 3] 2b** — share code / QR carrying the signed device list.
  Redundant for single-device (legacy fallback already synthesizes device 0);
  only meaningful once linking exists.
- **[moved to Phase 3] 2d** — receive-side verification of a sender's device
  against their stored list. Only reachable once device lists are distributed
  (a contact must have >1 device and the receiver must know the list), which is
  Phase 3 work.

### Phase 3 — Device linking / provisioning (now also carries 2b + 2d)
- QR link flow: primary signs the new device's cert, transfers state (contacts +
  device list + history seed via the encrypted export archive) over an
  authenticated LAN/Tor channel.
- **(2b)** share code / QR carries the signed device list; contact-add parses +
  verifies it.
- Push updated device list to all contacts (control message); store it via
  `save_contact_device_list` (already built in 2a).
- **(2d)** receive path accepts a message whose `from` is any device in the
  sender's verified list (not only the account key), and rejects devices not in
  the list.
- Unlink / revoke (remove device, bump version, re-sign, push).
- Resolve the "who can link" decision above.
- Tests: link a 2nd device in-harness; contacts learn it; a message from a
  linked device verifies; revoke removes it.

### Phase 4 — Self-sync replication (hardest)
- Device-to-device encrypted sync channel: sent-message echo, read receipts,
  contact/device-list changes; queued via retry when a device is offline;
  idempotent via `sync_seen_items`.
- Tests: send from D1 → D2 shows it; add contact on D1 → D2 gets it; conflict /
  out-of-order handling.

### Phase 5 — GUI + clients
- Linked Devices screen (list / link-via-QR / unlink); first-run "link to
  existing account"; unified conversation view across devices.
- Rebuild TUI / AppImage / APK.

### Phase 6 — Hardening
- Revocation propagation, device-list version-conflict resolution,
  adversarial-device-list tests, key-compromise handling.

## Non-negotiables (carried from the project brief)
- **Serverless** native core — devices sync peer-to-peer over Tor/LAN/BT, never
  a central server.
- One Rust core owns data/crypto/history; clients stay thin.
- Native wire stays E2E regardless of transport.
- Single-device accounts keep working unchanged at every phase.
