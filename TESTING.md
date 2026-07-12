# Testing Sideband across all three clients

Sideband ships three front-ends — the **TUI/CLI**, the **desktop GUI** (Flutter,
which shells out to the CLI), and the **Android GUI** (Flutter, which calls the
Rust core over `dart:ffi`). All three are thin shells over the **same Rust
core**, so most cross-client behaviour is core behaviour. The test strategy
exploits that: test the shared core hard and deterministically, then smoke-test
each client's thin shell.

Run everything with:

```bash
./run-tests.sh fast   # default — no Tor, no emulator, runs in seconds. CI gate.
./run-tests.sh ui     # fast + real Flutter UI driving (Linux, and Android if attached)
./run-tests.sh e2e    # the full two-peer test over real Tor (slow/flaky; nightly/manual)
./run-tests.sh all
```

## Why not "three real clients talking over Tor"?

You *can* run the emulator, TUI, and desktop GUI on one machine at once — that's
not the constraint. The constraints are that (1) the clients talk over **Tor**
(30–60s bootstrap, nondeterministic) and (2) driving GUIs by screen pixels is
flaky. So real-client-over-Tor is the **e2e** tier only: run it rarely. Almost
all coverage lives in the fast tier, which never touches Tor.

## The layers

### Fast tier (`run-tests.sh fast`) — the CI gate

- **Core + interop harness** (`cargo test`). [`src/interop.rs`](src/interop.rs)
  spins up isolated peers, each with its **own temp profile and identity**, and
  hand-delivers *real* signed+encrypted wire messages A→B through the actual
  build → encode → decode → verify → store pipeline — no Tor. It asserts on both
  the stored history and the `InboundMessage` events the UI would receive.
  Covers: message attribution to a known contact, unknown-sender
  trust-on-first-contact, the v3 double ratchet, and contact-add with STANDARD
  base64 keys (`+`, `/`, `=`).
  - Enabled by two testability seams: `build_outbound_message` (build the wire
    message without sending) and `handle_text_message` (the inbound text path,
    which needs no `TorClient`).
- **Android client contract** (`cargo test`, in [`src/api.rs`](src/api.rs)
  `mod tests`): calls the exact `sideband_api_*` FFI exports the app uses over
  `dart:ffi`, parses the JSON envelopes, and frees the strings — no emulator.
- **Desktop client contract** (`cli_contract` in
  [`run-tests.sh`](run-tests.sh)): runs the exact `sideband` subcommands the
  desktop GUI issues via `Process.run` (`contact add/list --json`,
  `history --json`, `group list --json`, `contact delete`) against a throwaway
  profile.
- **Dart logic** (`flutter test`): pure widget/logic tests, including
  `parseAddCommandContact` with `+`/`/`/`=` keys, attachment-path validation,
  notification helpers, transfer-string parsing, and the TUI QR-overlay render
  test.

### UI tier (`run-tests.sh ui`) — real UI driving

Uses Flutter's `integration_test` (widget-key/text finders, **not** pixel
coordinates — reliable where `xdotool` is not). The same test runs on
`-d linux` and, if a device/emulator is attached, `-d android`. This is where
client-specific glue (message-box routing, sidebar refresh, the add-contact
dialog) is exercised. *Scaffold pending — see below.*

### E2E tier (`run-tests.sh e2e`)

[`test_e2e.sh`](test_e2e.sh): two `serve` instances bootstrap real Tor onion
services and exchange a message. Slow and network-dependent; run nightly or by
hand, never per-commit.

## Profiles are always isolated

Every instance is defined by its `--profile` directory (identity keys,
`contacts.toml`, `messages.db`, ratchet state). The harness and the CLI-contract
test each create a fresh temp profile per peer and delete it afterwards, so runs
never touch `~/.sideband` or each other.

## Still to add

- `gui/integration_test/` for the UI tier (needs a "skip the Tor listener on
  launch" test hook so the add-contact / message-routing flows can be driven
  without bootstrapping Tor).
- A PTY-driven TUI test to complement the ratatui `TestBackend` render tests.
