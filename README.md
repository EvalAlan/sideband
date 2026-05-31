# Sideband

Experimental privacy-first, serverless chat over Tor onion services.

**Not production secure. Not peer-reviewed. Prototype status.**

## Features

- Tor onion-service transport via **Arti** (embedded Rust Tor, `arti-client`)
- Ed25519 identity + signed messages
- X25519 + ChaCha20-Poly1305 encryption
- Double Ratchet forward secrecy
- SQLite message history
- Ratatui TUI with contacts sidebar, message scroll, timestamps
- File transfer offers with encrypted metadata

## Build

### Prerequisites

- **Rust** (1.75+): [rustup.rs](https://rustup.rs)

### Build & verify

```bash
git clone https://github.com/yourname/sideband
cd sideband

# Compile
cargo build

# Or run directly
cargo run -- tui

# Check, test, format
cargo check
cargo test
cargo fmt --check
```

All 22 tests should pass. Tor is not required for `cargo test` — tests use temporary profiles without network.

## Quick start

Sideband stores your default profile in `~/.sideband`. If it does not exist, the CLI/TUI creates it on first run.

```bash
cargo run -- tui
```

Wait for the onion address to appear, then:

1. Share your info: type `/share` and send the output to your contact
2. Add their contact: paste the `/add` line they send you
3. Optional: initialize forward secrecy with `/ratchet <contact>`
4. Type messages and press Enter to send
5. Press Tab to cycle contacts, PgUp/PgDn to scroll

## CLI usage

The default profile is `~/.sideband`. Use `--profile <dir>` only for test profiles or multiple local identities.

```bash
# Show your identity and public keys
cargo run -- identity

# Run the onion-service listener
cargo run -- serve

# Add a contact
cargo run -- contact add \
  --name alice --onion <alice-onion>.onion \
  --pubkey <ed25519-b64> --x25519-pubkey <x25519-b64>

# Send a message
cargo run -- send --to alice --message "hello"

# Initialize double ratchet for forward secrecy
cargo run -- ratchet alice

# Test/development profile override
cargo run -- tui --profile ./profiles/alice
```

## TUI commands

| Command | Description |
|---|---|
| `/send <contact> <msg>` | Send a message |
| `/file <contact> <path>` | Send a file offer |
| `/history [contact]` | Show message history |
| `/contacts` | List contacts + ratchet status |
| `/add <name> <onion> <ed25519_pk> <x25519_pk>` | Add contact |
| `/delete <contact>` | Remove contact |
| `/name [display-name]` | Show or set your display name |
| `/whoami` | Show identity keys |
| `/share` | One-liner to share with a contact |
| `/onion` | Show onion address |
| `/ratchet <contact>` | Initialize double ratchet |
| `/status` | Full status: Tor, ratchet, contacts |
| `/help` | Show all commands |
| `/clear` | Clear message pane |
| `/quit` | Exit |

## Message protocol

Three versions, auto-negotiated:

| v | Encryption | Ratchet | Notes |
|---|---|---|---|
| 1 | Signed only, plaintext | No | Legacy fallback |
| 2 | X25519 + ChaCha20-Poly1305 | No | Static key agreement |
| 3 | Double Ratchet, X25519+HKDF+AEAD | Yes | Forward secrecy after `/ratchet` |

### JSON wire format, v3 example

```json
{
  "v": 3,
  "type": "chat_message",
  "from": "<ed25519-pubkey-b64>",
  "timestamp_ms": 1700000000000,
  "body": "",
  "sig_b64": "<ed25519-sig-b64>",
  "enc_body": "",
  "ratchet_header_b64": "<dh_pub | send_n | prev_n, base64>",
  "ratchet_nonce_hex": "<hex>",
  "ratchet_ct_hex": "<hex>"
}
```

Ed25519 signature covers all fields except `sig_b64` using sorted-key JSON.

## How it works

Sideband uses **[Arti](https://arti-project.org/)** (`arti-client` crate) as an embedded Tor implementation:

- `TorClient::create_bootstrapped()` connects to the Tor network
- `OnionService` publishes a v3 hidden service for inbound connections
- `TorClient::connect()` opens anonymized outbound connections to onion services
- No system `tor` binary required; all Tor functionality runs in-process
- Arti state is stored under `<profile>/arti_state/`

Encryption is handled in three layers:

1. **Transport**: Tor onion-service (onion-to-onion, end-to-end encrypted by Tor)
2. **Application v2**: static X25519 key agreement → ChaCha20-Poly1305 AEAD
3. **Application v3**: Double Ratchet (per-message forward secrecy via HKDF chain keys + DH ratchet steps)

Keys:
- **Ed25519** for message signing (authentication)
- **X25519** for encryption key agreement

## Profile structure

```text
~/.sideband/
  identity.toml          # Ed25519 + X25519 keys
  contacts.toml          # name -> {onion, pubkey, x25519_pubkey}
  messages.db            # SQLite history
  ratchet/<contact>.bin  # Double Ratchet state
  arti_state/            # Arti Tor client state
```

## Planned transport architecture

Sideband is moving to a **protocol core + transport adapters** model.

- Core protocol: identity, signing, encryption, ratchet, storage, framing
- Transport adapters: Tor, Meshtastic, MeshCore, future backends

Initial transport abstraction has been introduced in `src/transport/`:

- `Transport` trait
- `Envelope` with `msg_id`, `seq/total`, `ttl`, `hop_count`, `transport_hint`, `ack_for`
- `TransportCapabilities` and `TransportStatus`
- `TorTransport` adapter stub

This is scaffolding only; runtime behavior is unchanged until Tor send/recv paths are fully wired through the trait.

Current state:
- CLI `serve` and `send` now route through `TorTransport`
- TUI bootstrap/serve/send/file-offer paths route through `TorTransport`
- Core `Transport` trait exists but generic envelope send/recv is still stubbed for Tor

## Known limitations

- Arti bootstrapping on first run requires network access and may take 30-60s
- No Bluetooth or Wi-Fi Direct transports
- No mobile client
- File transfer currently sends offers only; chunk reassembly is TODO
- No delivery receipts beyond sent/failed
- No out-of-order message handling (no skip buffer in Double Ratchet)
