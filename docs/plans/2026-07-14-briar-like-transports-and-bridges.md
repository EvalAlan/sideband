# Briar-like Transports + Multi-network Bridges — Plan

> **STATUS: 📋 PROPOSED (not started).** Design/roadmap for (A) reworking
> Sideband's WiFi/LAN + Bluetooth to match Briar's privacy model, and (B) a
> later Beeper-style multi-network bridge layer with a shared inbox. For current
> shipped state and the assistant handoff protocol, read [`AGENTS.md`](../../AGENTS.md).

**Goal.** Make Sideband's short-range/offline transports (WiFi-LAN, Bluetooth)
work like Briar — *unlinkable, contact-only, carrier-independent* — instead of
the current identity-broadcasting LAN prototype. Then add an opt-in bridge layer
so non-Sideband networks appear in one shared inbox (Beeper-style), while keeping
the Sideband-native core strictly private.

---

## Part 0 — How Briar actually does it (findings, from source + specs)

Verified against Briar's specs and source (see Sources at the bottom). Some of
this corrects common lore (including our own earlier assumptions).

1. **Contacts are added out of band, then everything is contact-only.** Pairing
   is via QR/`briar://` link (Bramble QR Protocol, BQP) in person, or remotely
   via the **Bramble Rendezvous Protocol (BRP)** — which derives *pseudo-random
   Tor hidden-service addresses* from the pair's shared secret and polls once/min
   for up to 48h. BRP is **Tor-only** and is for *initial rendezvous*, not
   ongoing transport discovery. Briar never syncs with strangers.

2. **Addressing = exchanged "transport properties," not broadcast.** Each device
   shares per-transport properties **with its contacts over an existing encrypted
   connection**, and updates them as they change:
   - **LAN/WiFi:** its last-known LAN **IP:port(s)**. If two contacts are on the
     same WLAN and at least one knows the other's current LAN address from the
     shared property, they connect directly — no internet, **no broadcast of
     identity**. (Automatic broadcast/mDNS LAN discovery has historically been an
     open issue; the shipped mechanism is exchanged addresses.)
   - **Bluetooth:** its Bluetooth **MAC address** + a **stable, random, per-device
     128-bit UUID** (`contactConnectionsUuid`, generated once and stored as a
     transport property — it does **not** rotate). Contacts connect to that
     MAC using that RFCOMM service UUID. (A *derived* UUID from a commitment is
     used only for the ephemeral key-agreement/pairing handshake, not for
     established-contact connections.)

3. **Connection security = Bramble Transport Protocol (BTP)** wraps *every*
   connection over *every* carrier:
   - Per-contact root key → **time-period rotated** transport keys (period ≈ a
     few tens of seconds; "rotation mode" deletes the root for forward secrecy).
   - Each stream starts with a **pseudo-random 16-byte tag** (first bytes of a PRF
     over the tag key + stream number); the recipient **pre-computes expected
     tags** to recognize inbound streams. **No plaintext headers, no handshakes,
     no timeouts, optional padding** → traffic looks like random bytes and a
     passive observer can't link a stream to a contact or even fingerprint it as
     Briar.

4. **Sync = Bramble Synchronisation Protocol (BSP).** On any connection, contacts
   run a **bidirectional, delay-tolerant sync**, exchanging all pending messages
   both directions (dedup by message id). It is *not* "dial peer, stream one
   message." This is the store-and-forward mesh model.

**Takeaway:** Briar's privacy comes from (a) never broadcasting identity —
addresses/UUIDs are shared contact-to-contact as transport properties — and (b)
BTP making every wire connection unlinkable and unrecognizable, over any carrier;
plus (c) a sync model rather than one-shot sends.

---

## Part 0.1 — Where Sideband is today (gap analysis)

| Aspect | Sideband today | Briar | Gap |
|---|---|---|---|
| LAN discovery | **Broadcasts a signed beacon containing our Ed25519 pubkey in the clear** to the whole LAN | Exchanges IP:port with contacts as transport properties; no broadcast | **Big** — we leak stable identity/presence to the local network |
| LAN connection | E2E-encrypted `ChatMessage` over **plain TCP** | BTP-wrapped (tags, no headers, padding, FS) | **Big** — our LAN traffic is recognizable + linkable by size/timing |
| Send model | Dial peer, send one message (LAN fast-path, else Tor) | Bidirectional sync of all pending per connection | Medium — we have an offline outbox but no per-connection sync |
| Bluetooth | none | RFCOMM via MAC+UUID transport properties | Missing carrier |
| Transport selection | Tor-direct + a LAN special-case | Pluggable plugin registry | Medium — not yet a real registry |
| Pairing | QR/`/add` link (in person / copied) | BQP + BRP (remote rendezvous) | BRP-style remote rendezvous is a nice-to-have |

Good news: the crypto-independent-of-carrier principle already holds (messages
are E2E signed+encrypted), and we already have the pieces to build on — the
`Transport` trait, one shared `handle_inbound` dispatch, the offline outbox +
retry queue, the per-contact ratchet/static keys, and `seen_messages` dedup.

---

## Part A — Make WiFi + Bluetooth Briar-like

Ordered so each phase is independently shippable and de-risks the next. Keep Tor
as the always-available fallback throughout.

### A1. Transport properties + contact-to-contact address exchange *(replaces the LAN pubkey broadcast — highest-priority privacy fix)*
- New signed typed message `transport_props` (same pattern as receipts): a peer
  shares its current reachable addresses **only with its contacts**, over any
  existing connection (Tor/LAN), and refreshes on change.
  - LAN: last-known LAN `IP:port(s)`.
  - Bluetooth (A4): MAC + per-device UUID.
- Persist per contact in a `contact_transport_props(contact, transport, value,
  updated_at)` table.
- Send path resolves a contact's LAN address from **their shared property**, not
  from a broadcast registry.
- **Remove/deprecate** the open `LanBeacon` pubkey broadcast. Replace default LAN
  discovery with address-exchange. *(Optional, off by default:* a privacy-
  preserving local discovery that broadcasts only a **per-contact rotating token**
  — a truncated PRF over the pair's shared secret + time period — so only a
  contact recognizes it and no identity is exposed; useful for dynamic-IP LANs.
  This is strictly better than today's pubkey beacon and mirrors Briar's
  "pseudo-random details known to both peers.")

### A2. BTP-lite: wrap non-Tor connections
- A link-layer wrapper for LAN (and later BT) connections, keyed by a per-contact
  **transport root** (derive from the existing static shared key or ratchet root
  via HKDF with a distinct label; do **not** reuse message keys):
  - Time-period key rotation (forward secrecy).
  - Per-stream **pseudo-random 16-byte tag**; recipient pre-computes expected tags
    per contact to recognize/attribute an inbound stream with no plaintext header.
  - Authenticated-encrypted frames, **optional padding**, no timeouts/handshakes.
- Result: Sideband LAN/BT traffic is unrecognizable and unlinkable to a passive
  local observer — the property Briar gets from BTP.
- Tor already provides its own stream security; BTP-lite applies to LAN/BT.

### A3. Sync model (BSP-lite)
- On an established LAN/BT (and optionally Tor) connection, run a **bidirectional
  sync**: exchange all *pending/undelivered* messages both directions, dedup by
  the existing replay fingerprint / a message id. Delay-tolerant.
- Unifies with the offline outbox (A message queued while offline flushes on the
  next sync with that contact over any carrier) and sets up the future
  store-and-forward **mailbox/mesh** (multi-hop via `Envelope.hop_count/ttl`).

### A4. Bluetooth transport (the hard carrier)
- Model on Briar: advertise an RFCOMM server socket under a **per-device UUID**
  (shared via A1 transport properties); connect to a contact's **MAC + UUID**.
- **Android:** Kotlin BT APIs (classic RFCOMM first; BLE is harder — MTU,
  background limits) bridged to the Rust core via MethodChannel/FFI. Needs BT
  permissions + a foreground service.
- **Desktop (Linux):** optional, via a Rust BT stack (e.g. `bluer`/BlueZ).
- Carries BTP-lite frames + BSP-lite sync, same as LAN.

### A5. Pluggable transport registry
- Formalize the `Transport` trait into a registry the core iterates: inbound is
  merged from all active transports into one `handle_inbound` dispatch (already
  true for LAN today); outbound tries transports by **preference/reachability**
  (LAN/BT when a fresh address is known, else Tor). Replaces the current
  Tor-direct + LAN-fast-path special case.

### A6. (Optional) BRP-style remote rendezvous
- Add contacts remotely without copying a link, by deriving pseudo-random Tor
  rendezvous addresses from a pre-shared secret and polling — Briar's BRP. Lower
  priority; the QR/`/add` flow already works.

### Also fix now (small): Android LAN multicast lock
- Even for the current beacon, Android needs a `WifiManager` multicast/broadcast
  lock for UDP to work. (Moot if A1 removes the broadcast, but note it.)

---

## Part B — Beeper-style multi-network bridges + shared inbox *(later; separate track)*

Goal: let a user see and use **other chat networks** (WhatsApp, Signal, Telegram,
XMPP, IRC, SMS, iMessage, …) inside Sideband — one **shared inbox**, switch
between platforms — the way Beeper does. Beeper is built on **Matrix + mautrix
bridges** (each network has a bridge that translates it to Matrix; the client
aggregates all Matrix rooms).

### Hard privacy boundary (non-negotiable, state it everywhere)
- **Sideband-native (Tor/LAN/BT) is the private core** — E2E, serverless,
  metadata-resistant. **Bridged networks are NOT** — they traverse third-party
  servers and bridge infrastructure and carry those networks' privacy properties
  (often none). Bridges are **opt-in**, clearly **labeled per conversation**, and
  isolated from the native trust domain. Never let a bridge silently downgrade a
  conversation's security or blur which network a message is on.

### B1. Bridge/connector abstraction
- A `Bridge` trait (network id, capabilities, auth/login, send, receive-stream,
  contact/room list). Connectors run as **sidecar processes** the Sideband core
  talks to over a local socket — so heavy/AGPL/third-party bridge code stays out
  of the core process.
- **Recommended first path:** reuse the mature **mautrix** bridge ecosystem via an
  embedded/local Matrix homeserver (e.g. Conduit/Dendrite) — the core becomes a
  Matrix client aggregating bridged rooms. Alternative: native protocol
  connectors per network (more control, far more work). Start with 1–2 bridges
  (e.g. Signal, Telegram, or XMPP/IRC) to prove the abstraction.

### B2. Unified inbox data model
- A `conversation` gains a `network` (native | matrix:<bridge>) + external
  identity mapping; the inbox is one list across native + bridged, each tagged
  with a network badge. Unified search. History persisted with the network tag.
- Map external contacts ↔ a local address-book entry without conflating them with
  cryptographic Sideband contacts.

### B3. Shared-inbox UI
- One conversation list across all networks with per-network filter/switch and
  clear badges; a unified compose box that routes to the conversation's network;
  account/login management per bridge; a visible **security indicator** (private
  native vs bridged) on every conversation.

### B4. Routing + identity
- Outbound routes by the conversation's network. Optional: a single Sideband
  identity fronting multiple bridged accounts. Keep native and bridged send paths
  strictly separate in code.

---

## Non-negotiables

- No central server for the native core. Bridges are external/opt-in and never
  part of the native trust domain.
- Do not fork the message stack per client — Rust core owns transports, crypto,
  sync, and history; clients stay thin.
- Every wire carrier stays **E2E-encrypted regardless of carrier**; non-Tor
  carriers additionally get BTP-lite so they're unlinkable/unrecognizable.
- Default to privacy: LAN/BT discovery and every bridge are **opt-in**.

## Testing
- Deterministic, no-radio unit/interop tests (like `src/interop.rs` /
  `transport::lan` tests): transport-property exchange + address resolution;
  BTP-lite tag pre-computation + frame round-trip + unlinkability (tags look
  random); BSP-lite two-peer sync converges + dedups; registry transport
  selection. Radios (BT, real LAN, bridges) get thin smoke tests behind a manual
  tier.

## Open questions
- BTP-lite: derive transport keys from the existing ratchet root, or a separate
  transport-key-agreement? (Briar has a dedicated transport key manager.)
- Sync scope: sync *all* undelivered, or a windowed/most-recent set? Interaction
  with disappearing messages (don't sync already-expired).
- Bridges: mautrix-via-Matrix (breadth, heavier) vs native connectors (control,
  slower). Which 1–2 networks first?

## Sources
- Bramble Transport Protocol (BTP): https://code.briarproject.org/briar/briar-spec/-/raw/master/protocols/BTP.md
- Bramble Rendezvous Protocol (BRP): https://code.briarproject.org/briar/briar-spec/blob/master/protocols/BRP.md
- Briar "How it works": https://briarproject.org/how-it-works/
- How Briar Connects to Contacts (community wiki): https://notabug.org/rouch/Briar/wiki/How-Briar-Connects-to-Contacts
- Briar source — `AbstractBluetoothPlugin` (contactConnectionsUuid is a stored random per-device UUID; commitment-derived UUID only for key agreement).
- Beeper/mautrix bridge model (Matrix bridges).
