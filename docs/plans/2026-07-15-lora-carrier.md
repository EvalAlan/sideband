# LoRa carrier — design spec

> **STATUS: 📋 PROPOSED (not started).** A new off-grid radio carrier for
> Sideband, hanging off the existing transport layer. For shipped state + the
> handoff protocol read [`AGENTS.md`](../../AGENTS.md).

## Context

Sideband is at its best when it needs **no infrastructure**: serverless, E2E,
contact-only, works when the internet doesn't. LoRa extends that into genuine
off-grid range (km-scale, no cell/WiFi/Tor) — the same identity, just a longer,
slower pipe. This is a far better fit than the (now-parked) Beeper bridges, which
pulled toward central infrastructure.

The goal: **send/receive Sideband text messages over LoRa radio, with the same
E2E crypto and contact-only privacy as every other carrier**, as an opt-in
transport that activates when a LoRa modem is attached. Not a replacement for
Tor/LAN/BT — an additional carrier for when nothing else is reachable.

## What already fits (why this is mostly plumbing, not new architecture)

The transport layer was built carrier-agnostic and already anticipates this:

- **`Transport` trait** (`src/transport/mod.rs`): `name/local_addr/capabilities/
  status/send/try_recv`. A LoRa carrier is one more impl.
- **`Envelope`** already has `seq`/`total` (**fragmentation**), `ttl` +
  **`hop_count`** (**mesh is anticipated**), and `ack_for`.
- **`TransportCapabilities { max_payload, supports_chunking, … }`** lets a carrier
  advertise a tiny MTU and lean on the core's chunking.
- **`TransportRegistry`** (`src/transport/registry.rs`) is priority-ordered, so
  LoRa registers at **low priority** — a last-resort carrier chosen only when
  LAN/BT/Tor are unreachable. Add `RouteEndpoint::Lora(..)`.
- **E2E is already carrier-independent**, and non-Tor carriers already get
  **BTP-lite** unlinkable framing (`src/transport/btp.rs`).
- **Platform-radio-as-byte-pump precedent**: the Android Bluetooth adapter is a
  byte-pump reached over a **private Unix-domain socket**
  (`src/transport/bluetooth.rs`, `spawn_bridge_server`). The LoRa modem shim uses
  the exact same pattern.

So the core (identity, ratchet, sync, history, chunking, mesh fields) is done.
LoRa adds: a modem shim, a compact wire profile, and carrier wiring.

## The two real constraints

1. **Bandwidth is minuscule.** LoRa is ~0.3–27 kbps raw; a frame is ~**222–255 B**
   (Meshtastic caps payload ≈237 B); regions impose **duty-cycle limits** (e.g.
   1% on EU868 — after a long transmission you must stay silent ≈99× as long).
   Text only. No files, no media, no presence spam.
2. **Bring-your-own-radio.** Needs hardware: a USB/serial LoRa board on desktop,
   or a BLE-paired board on Android. Pragmatic target: **use a Meshtastic device
   as a dumb pipe** (send/receive raw payloads; layer Sideband's own E2E on top —
   strictly stronger than Meshtastic's channel PSK).

## The gating design problem: message size

BTP-lite framing is **~76 B/stream** (`PREFIX_AND_HEADER_LEN` = 40 B prefix + 36 B
header), and the inner `ChatMessage` is **signed (Ed25519 = 64 B) + ratchet +
AEAD-tagged (16 B) JSON** — easily 200–400 B for even a short text. Over a ~222 B
LoRa frame that's 2+ fragments and seconds of airtime per message. It works via
`Envelope` chunking, but it's wasteful.

**So the real prerequisite is a compact binary wire profile** (call it
**BTP-micro**) for constrained carriers — which *also* makes LAN/BT leaner:

- **Binary `ChatMessage`, not JSON** — fixed field order, varints, no keys.
- **Trim BTP-lite for small MTUs** — shorter contact tag (e.g. 8 B vs 16 B; tune
  the unlinkability/collision trade), drop redundant header fields, single-frame
  fast path.
- **Signature question (needs crypto review):** for an *established ratchet*
  session the ChaCha20-Poly1305 AEAD already authenticates the sender via shared
  ratchet state, so the extra **Ed25519 signature (64 B) may be redundant** on
  constrained carriers. Dropping it for ratchet messages ~halves a short-text
  frame. This is a real win but a crypto-review-gated change — spec it, don't
  assume it.
- Budget target: a short text (≤~120 chars) in **one** LoRa frame.

## Privacy: LoRa's broadcast medium is a *feature* here

LoRa is broadcast; everyone in range hears every frame. BTP-lite already makes
each frame an **unlinkable pseudo-random tag** only the intended contact
recognizes — so on the air, observers can't tell who is talking to whom, and
there are no plaintext headers or addresses. That's a *better* privacy story than
Meshtastic's shared-PSK channels, and it means LoRa needs **no explicit
addressing**: broadcast the tagged frame; the right peer recognizes + decrypts it
(everyone else fails the tag check cheaply). Contacts optionally exchange a LoRa
node id via the existing **A1 transport-property exchange** to scope/relay, but
delivery does not require it.

## Mesh / range beyond one hop

`Envelope.ttl` + `hop_count` already exist. **Start single-hop** (direct radio
range). Multi-hop is a later, separate piece: opportunistic relay where nodes
rebroadcast tagged frames they can't decrypt (decrement `ttl`, bump `hop_count`,
dedup by `msg_id`). This overlaps the parked **store-and-forward** backlog item
and needs its own flooding/loop-prevention + airtime-fairness design. Not P1.

## Component design

### Modem shim (mirrors the Bluetooth bridge)
A small per-platform byte-pump between Sideband and the radio, reached over a
private Unix-domain socket — same shape as `bluetooth::spawn_bridge_server`:
- **Desktop:** a serial reader/writer (USB LoRa board or a Meshtastic device over
  serial). Likely a tiny sidecar (`sideband-lora-modem`) speaking the Meshtastic
  serial API or raw KISS, framing payloads to/from the socket.
- **Android:** a Kotlin BLE shim (pair with a Meshtastic board), same socket
  contract — reuses the Bluetooth bridge plumbing.
The core never talks to hardware directly; it reads/writes framed payloads on the
socket, exactly as it does for Bluetooth.

### Carrier (`src/transport/lora.rs`)
- Implements `Transport`: `capabilities()` advertises `max_payload ≈ 200`,
  `supports_chunking = true`, `supports_streaming = false`.
- `send()`: BTP-micro-frame the `Envelope`, fragment to the MTU, **pace to the
  region duty cycle** (a token-bucket airtime governor — this is mandatory, not
  optional), write frames to the modem socket.
- `try_recv()`: read frames from the socket, reassemble by `msg_id`/`seq`,
  BTP-micro-verify, emit `Envelope`s into the same inbound channel as every other
  carrier (`inbound_sender()`), so `serve()`'s dispatch loop is unchanged.
- Register `RouteEndpoint::Lora` at low priority; wire into `serve()` next to the
  LAN/BT setup, gated on a `lora_enabled` profile toggle (off by default).

## Phased plan

- **P0 — BTP-micro compact wire profile** *(prerequisite; benefits all carriers)*.
  Binary `ChatMessage` + trimmed framing + the signature-drop decision (crypto
  review). Deterministic round-trip + size tests. **Do this first.**
- **P1 — LoRa carrier + desktop serial modem shim.** Single-hop, text-only,
  duty-cycle governor. Deterministic tests with a **fake modem** over the socket
  (like `sideband-bridge-login-mock` / the BT fake): frame → fragment → pace →
  reassemble → verify, no radio.
- **P2 — Android BLE modem shim** (reuses the Bluetooth bridge pattern).
- **P3 — opportunistic multi-hop relay** (ttl/hop_count flooding + dedup +
  airtime fairness; ties into store-and-forward).

## Testing
- **Deterministic, no radio** (the bar for `run-tests.sh fast`): BTP-micro
  frame/parse round-trip + size assertions; fragmentation/reassembly across a
  200 B MTU; duty-cycle governor pacing (mock clock); a fake-modem two-peer
  exchange over the socket (mirrors `src/interop.rs`).
- **Manual radio tier** (behind `run-tests.sh` e2e/manual): two real boards,
  a text round-trip, a range/loss check, duty-cycle compliance.

## Non-goals
- Files/media/large payloads over LoRa. High throughput. Replacing Tor/LAN/BT.
- Meshtastic's own mesh/routing (we use the device as a dumb pipe; if we later
  relay, it's Sideband-tagged frames, not Meshtastic channels).

## Open questions
- **Modem protocol:** drive a Meshtastic device via its serial/BLE API (fast to
  ship, cheap hardware) vs. a raw SX127x/SX126x board via KISS (more control).
  Recommend Meshtastic-as-modem for P1.
- **Region/duty-cycle config:** how the user sets their region (EU868/US915/…);
  affects MTU (SF) + airtime budget. Needs a small setting.
- **Signature drop** on constrained carriers — crypto review required before P0
  commits to it.
- **Tag length** for BTP-micro (8 B vs 16 B): unlinkability + collision vs airtime.

## Prior art to study (and a name collision to note)
- **Meshtastic** — LoRa mesh + cheap ESP32/SX12xx hardware; the modem target.
- **Reticulum / LXMF** (Mark Qvist) — encrypted serverless messaging over LoRa/
  packet radio; its GUI client is *also named "Sideband"* (real prior art + a
  naming overlap to be aware of). Closest existing "Sideband-over-LoRa"; worth
  studying for framing + possible interop.
