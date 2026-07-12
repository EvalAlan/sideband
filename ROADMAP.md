# Sideband 1.0 Roadmap

## Collaboration workflow

This file is the shared source of truth for roadmap work across GPT, Claude, and human contributors. Before starting a milestone:

1. Read this roadmap and inspect the current repository state.
2. Preserve unrelated working-tree changes.
3. Work from the earliest incomplete prerequisite rather than cherry-picking later architecture.
4. Add or update tests with each behavior change.
5. Update this file when scope, sequencing, or an architectural decision changes.
6. Do not mark work complete based only on code generation. Record the verification command and result in the commit or pull request.

Roadmap items describe intended direction, not evidence that a feature already exists. The repository and passing tests remain authoritative.

## Goal

Ship a trustworthy, supportable Sideband 1.0 as an Android-first, Tor-based decentralized messenger with a Linux AppImage client. Prepare clean seams for local Bluetooth/Wi-Fi transport and post-1.0 external providers without prematurely mixing Gmail, radio protocols, or provider semantics into Sideband's encrypted protocol.

The target is product and security credibility, not a version-number ceremony.

## Recommended 1.0 scope

### Supported and stable

- Android application, primary platform.
- Linux x86_64 AppImage, secondary platform.
- Direct text messaging over Tor.
- Contact exchange, explicit trust state, verification, block/unblock.
- Durable offline queue, retry, duplicate suppression, restart recovery.
- Accurate delivery states.
- Attachments only if authenticated transfer hardening and quotas land.
- Groups only if an authenticated membership/administration model lands.
- Profile backup, restore, and forward migration.

### Experimental or deferred

- TUI/CLI remains supported for diagnostics and interoperability, but is not the primary consumer product.
- macOS, Windows, iOS, aarch64 AppImage, and Play Store publication are not claimed until each has a tested release pipeline.
- Bluetooth and local Wi-Fi are recommended for 1.1, after the core engine and transport seam are trustworthy.
- Gmail and other account providers are post-1.0 connectors.
- Meshtastic and MeshCore are later experimental constrained transports.

If Bluetooth/Wi-Fi parity with Briar is a hard definition of 1.0, move the optional Local Transport milestone into the release-critical path. Do not start it before protocol, persistence, and Android lifecycle work is complete.

## Current state

### What is already valuable

- Embedded Arti Tor transport and onion service in `src/transport/tor.rs`.
- Ed25519 identity, X25519/ChaCha20-Poly1305 static encryption, and a custom ratchet implementation in `src/main.rs`.
- SQLite history, retry queue, replay tracking, groups, and restart-safe transfer state.
- Deterministic peer interoperability tests in `src/interop.rs`.
- Android C ABI contract tests in `src/api.rs`.
- Flutter Android and desktop UI in `gui/lib/main.dart`.
- Unified builds in `build.sh` and tiered tests in `run-tests.sh`.
- Current Rust verification observed during assessment: 168 tests passed, strict Clippy passed, and formatting passed.

### Release blockers found in code

1. `src/main.rs` is about 7,200 lines and is included into the library by `src/lib.rs`. Protocol, storage, crypto, groups, transfer, CLI, and orchestration invariants are tangled.
2. The custom ratchet lacks a skipped-message-key cache and safely usable out-of-order semantics (`src/main.rs:3509-3821`; documented at `README.md:191`).
3. Ratchet state advances before network delivery and is stored independently from the outbound message/database transaction.
4. Inbound authentication failure is often treated as metadata rather than authorization failure. Unverified content can affect messages, groups, files, and ACK state in `src/handler.rs`.
5. Group membership and administration are sender-supplied fan-out metadata without a signed epoch/revision or enforceable owner/admin authorization.
6. v1 signed plaintext remains accepted and downgrade policy is not pinned per contact.
7. The `Transport` trait in `src/transport/mod.rs` exists, but Tor types and onion addresses still leak through handlers, contacts, file ACKs, and orchestration.
8. Android's foreground service does not own the Rust/Tor listener. The listener still lives in the Flutter process, so reliable background receive and process-death recovery are not established.
9. Android calls handwritten synchronous FFI from the Flutter UI isolate. Some shared actions still fall through into desktop `_Cli` paths.
10. Storage has ad hoc migrations but no explicit profile/schema version, migration transaction, backup, or newer-version rejection.
11. `test_e2e.sh` targets the old system-Tor/profile layout and does not assert a real current Arti exchange.
12. CI builds AppImages but does not run the complete fast gate, and there is no PR workflow or signed release workflow.
13. Android still uses `com.evalalan.sideband`; release builds can fall back to debug signing.
14. Product onboarding, safety-number verification, key-change handling, backup/restore, notification privacy, and lifecycle tests are incomplete.

## Architecture direction

Do not rewrite everything into crates before fixing behavior. Extract small modules incrementally while preserving tests.

### Core domains

- `identity`: `PeerId`, identity keys, trust state, contact records, key changes.
- `protocol`: typed/versioned packets, canonical encoding, signing, encryption, session state, limits.
- `store`: schema migrations, messages, dedupe, session state, outbox, transfers.
- `engine`: accepts app commands and inbound encrypted packets, applies protocol/store rules, emits domain events.
- `transport`: sends/receives opaque encrypted packet bytes and reports adapter status.
- `client adapters`: CLI/TUI, desktop subprocess bridge, Android native bridge, Flutter UI.

Suggested eventual paths:

- `src/identity.rs`
- `src/protocol/mod.rs`
- `src/protocol/session.rs`
- `src/store/mod.rs`
- `src/store/migrations.rs`
- `src/engine.rs`
- `src/transport/mod.rs`
- `src/transport/tor.rs`

Do not create all files at once. Extract one tested boundary per milestone.

### Peer and endpoint model

Replace the conceptual onion-centric contact model with:

```text
Contact {
  peer_id
  identity_key
  trust_state
  endpoints: [ProviderEndpoint]
}
```

For 1.0, only `TorEndpoint` is implemented. This allows later Bluetooth/Wi-Fi/Meshtastic routing without changing identity or chat semantics.

### Transport contract

Replace the current synthetic mesh-like `Envelope` as the universal abstraction with:

- `WirePacket(Vec<u8>)`: immutable encrypted Sideband protocol bytes.
- `Endpoint`: transport-specific address.
- lifecycle methods: start, send, status, shutdown.
- bounded inbound event channel rather than `try_recv()` polling.

Transport adapters do not receive plaintext, contact names, ratchet state, SQLite handles, or profile paths.

Fragmentation, RSSI, duty cycle, hop limits, and adapter ACKs stay inside constrained adapters unless the engine has a demonstrated reason to consume them.

### External provider boundary

Gmail is not a Sideband transport. It is an account provider with its own identity, threads, delivery semantics, OAuth credentials, and storage namespace.

```text
Unified UI conversation
  ├── SidebandConversation -> Sideband engine/protocol/transports
  └── ProviderConversation -> Gmail/provider connector and provider store
```

Never encode Gmail messages as Sideband `ChatMessage`, place them in Sideband ratchets, or treat OAuth tokens as profile/contact data.

## Milestone 0: Freeze the promise and threat model

Goal: decide what Sideband 1.0 actually guarantees before changing protocol code.

Tasks:

1. Write `docs/security/threat-model.md` covering:
   - malicious unknown peers;
   - compromised contacts;
   - network observers;
   - replay, reordering, duplication, and loss;
   - stolen/unlocked devices;
   - metadata leakage;
   - local database/key compromise;
   - group authorization;
   - attachment abuse and resource exhaustion.
2. Write `docs/protocol/compatibility.md` defining supported wire/profile versions and downgrade behavior.
3. Decide the ratchet path:
   - preferred: adopt a reviewed session protocol/library with interoperable vectors;
   - fallback: make 1.0 static authenticated encryption only and remove claims of forward secrecy.
4. Decide whether groups and attachments are stable 1.0 features or explicitly experimental.
5. Define platform support: Android and Linux x86_64 unless other platforms meet gates.
6. Remove contradictory `production-ready` claims from `README.md` until the gates are met.

Exit criteria:

- Security and product promises are written and testable.
- No plaintext v1 default/fallback is part of the normal 1.0 path.
- Group/attachment scope is explicit.

Likely files:

- `README.md`
- `docs/security/threat-model.md`
- `docs/protocol/compatibility.md`
- `Cargo.toml`
- `gui/pubspec.yaml`

## Milestone 1: Protocol and trust hardening

Goal: authenticate before dispatch and establish stable identity semantics.

Tasks:

1. Introduce a typed, versioned packet enum instead of free-form message-type strings.
2. Define exact canonical encoding, protocol domain separation, signed fields, AEAD associated data, message IDs, limits, and unknown-version behavior.
3. Publish golden encode/decode/signature vectors.
4. Replace `(plaintext, verified)` with an explicit inbound result:
   - authenticated;
   - pending identity;
   - duplicate;
   - invalid.
5. Permit only authenticated packets to mutate groups, transfers, files, ACK state, normal history, or delivery receipts.
6. Introduce stable `PeerId` derived from identity key material. Stop using mutable contact names as authority.
7. Add explicit trust states:
   - discovered/pending;
   - TOFU pinned;
   - user verified;
   - key changed/blocked.
8. Add minimum protocol version pinning per established contact and reject silent downgrade.
9. Add connection/read timeouts, semaphores, aggregate memory limits, transfer quotas, and authenticated-offer requirements.

Tests:

- Invalid signatures cannot store normal messages, mutate groups, write files, or advance ACKs.
- Duplicate/replayed packets are idempotent.
- Downgrade attempts fail after stronger protocol use.
- Oversized, slow, malformed, and chunk-without-offer traffic is rejected within bounded resources.
- Golden vectors decode identically across releases.


Likely files:

- `src/main.rs`
- `src/handler.rs`
- `src/types.rs`
- `src/interop.rs`
- new `src/identity.rs`
- new `src/protocol/`

## Milestone 2: Durable sessions, outbox, and migrations

Goal: make sends and upgrades crash-safe.

Tasks:

1. Add a profile/schema version and ordered migration runner.
2. Run each migration once inside a transaction.
3. Enable `PRAGMA foreign_keys=ON`.
4. Back up the profile/database before migration and reject profiles from newer unsupported versions.
5. Version ratchet/session serialization independently.
6. Introduce a durable outbox containing immutable already-encrypted packets.
7. Commit session advance, outbound packet, and local message state atomically.
8. Retry the same ciphertext/message ID rather than re-encrypting and consuming new session keys.
9. Commit inbound session advance, dedupe record, message/transfer mutation, and receipt atomically.
10. Add per-session locking and atomic config/contact writes.
11. Add profile integrity check, backup/export, restore/import, and corruption recovery.

Tests:

- Crash at every write boundary and restart without duplicate/lost messages.
- Sender offline/receiver offline retry using the same packet ID.
- Concurrent sends cannot reuse or overwrite session counters.
- Migration fixtures from every historical schema.
- Interrupted migration rolls back or safely resumes.
- Newer-profile version is rejected without mutation.
- `PRAGMA integrity_check` and `foreign_key_check` pass.

Likely files:

- `src/main.rs`
- `src/api.rs`
- new `src/store/mod.rs`
- new `src/store/migrations.rs`
- profile fixture directories under `tests/fixtures/`

## Milestone 3: Session protocol and delivery semantics

Goal: provide honest end-to-end security and message state.

Tasks:

1. Replace or deliberately remove the custom incomplete ratchet.
2. If retained/replaced, support bounded skipped-message keys, out-of-order delivery, duplicates, simultaneous session establishment, and restart.
3. Move all message types, including attachments/control packets, through the same authenticated session policy unless explicitly documented otherwise.
4. Define user-visible states:
   - queued;
   - submitted to transport;
   - peer authenticated and persisted;
   - failed retryable;
   - failed permanent.
5. Add authenticated peer receipts with stable message IDs if the UI uses “delivered.”
6. Remove fake presence. Use explicit transport reachability/activity language with expiry.

Tests:

- Loss, reorder, duplicate, delayed delivery, restart, and simultaneous send.
- Packet retry does not consume a second session key.
- Receipt spoofing/replay fails.
- UI state maps exactly to persisted protocol state.

Likely files:

- `src/protocol/session.rs`
- `src/handler.rs`
- `src/main.rs`
- `src/interop.rs`
- `gui/lib/main.dart`

## Milestone 4: Authenticated groups and hardened attachments

Goal: either make these stable or remove them from the 1.0 promise.

### Groups

1. Define immutable `GroupId` and stable member `PeerId`s.
2. Define creator/admin identity, signed membership operations, epoch/revision, and authorization rules.
3. Reject stale, forged, or unauthorized rename/add/remove/delete operations.
4. Define leave/delete semantics honestly. Local deletion is not remote revocation.
5. Migrate old local fan-out groups or retain them under an explicit legacy/experimental label.

### Attachments

1. Require authenticated offers before chunks.
2. Enforce file, chunk, transfer, sender, and total-storage quotas.
3. Verify decoded size and SHA-256 before atomic final rename.
4. Persist progress incrementally without rewriting all chunks.
5. Use provider capabilities so constrained transports can reject attachments before send.

Exit criteria:

- Group authority survives malicious member tests.
- Unauthenticated data cannot create files or advance transfers.
- File transfer survives interruption/restart and respects quotas.

Likely files:

- `src/handler.rs`
- `src/main.rs`
- `src/api.rs`
- `gui/lib/main.dart`

## Milestone 5: Honest Tor transport seam

Goal: make Tor the first adapter rather than part of the protocol core.

Tasks:

1. Remove `TorClient` and `TorTransport` types from protocol/handler APIs.
2. Move all Tor dialing, onion-service lifecycle, and address validation into `src/transport/tor.rs`.
3. Make handlers emit outbound protocol packets addressed to `PeerId`; the engine resolves an endpoint/provider.
4. Replace `SIDEBAND_REPLY_ONION` process-global routing state with engine/profile state.
5. Replace `try_recv()` polling with a bounded inbound channel.
6. Add an in-memory fake transport and transport conformance suite.
7. Make capability claims accurate. File ACK support is not generic message-delivery support.
8. Keep Tor as the only enabled production adapter for 1.0.

Conformance tests:

- lifecycle start/status/shutdown;
- exact max-payload behavior;
- disconnect/backpressure;
- malformed inbound frames;
- duplicate/out-of-order injection;
- cancellation;
- no plaintext/profile path leakage.

Likely files:

- `src/transport/mod.rs`
- `src/transport/tor.rs`
- `src/handler.rs`
- `src/main.rs`
- new `src/engine.rs`

## Milestone 6: Android reliability and client parity

Goal: Android receives messages reliably independent of Flutter activity lifetime.

Tasks:

1. Make the native Android foreground component own/supervise the Rust listener rather than merely displaying a notification.
2. Recover listener state after process/service restart.
3. Reconcile persisted inbound messages when Flutter returns.
4. Deep-link message notifications into the correct conversation.
5. Cancel only message notifications, never the listener foreground notification.
6. Validate foreground-service type and battery behavior on current Android versions.
7. Move blocking FFI/database/network work off the Flutter UI isolate using a worker isolate or a properly asynchronous native bridge.
8. Make refresh single-flight and cancellable.
9. Fix all FFI allocation ownership, including group-history branches.
10. Introduce one client backend interface. UI widgets and slash commands must not call `_Cli` or `_MobileApi` directly.
11. Remove every reachable Android fallback to desktop `Process.run`.
12. Implement Android parity for display name, identity, clear history, groups, transfers, and settings through the mobile backend.
13. Split `gui/lib/main.dart` by responsibility after backend interfaces are in place, not as a blind cosmetic refactor.

Acceptance tests:

- Background activity, swipe-away/process death, inbound message, persisted history, notification, and relaunch.
- Tor bootstrap while the UI remains responsive.
- Thousands of refreshes without overlap or native allocation growth.
- Every shared action produces equivalent backend state on Android and desktop.
- Minimum supported and current Android API levels on emulator plus at least two physical-device classes.

Likely files:

- `gui/lib/main.dart`
- `gui/android/app/src/main/kotlin/com/evalalan/sideband/ListenerForegroundService.kt`
- `gui/android/app/src/main/kotlin/com/evalalan/sideband/MainActivity.kt`
- `gui/android/app/src/main/AndroidManifest.xml`
- `src/api.rs`
- `src/app_api.rs`

## Milestone 7: Consumer security UX and product completeness

Goal: make protocol guarantees understandable and daily use credible.

Tasks:

1. Multi-step onboarding:
   - security/experimental notice;
   - display name and identity creation;
   - Tor bootstrap explanation/progress;
   - background/notification rationale;
   - backup warning;
   - add/share first contact.
2. Contact verification:
   - safety number/fingerprint;
   - QR compare/confirm screen;
   - explicit review before saving imported contact;
   - blocking key-change warning and re-verification.
3. Separate UI states for identity verification, encryption mode, session health, and transport reachability.

4. Pending-contact request inbox with Accept/Block.
5. Profile backup/export and restore/import before finalizing Android package identity.
6. Persistent user settings.
7. Notification privacy: sender-only/no body, lock-screen behavior.
8. App lock/biometric option and recents/screenshot privacy decision.
9. Search, paginated history, and durable event cursor reconciliation.
10. Accessible labels, dynamic text, screen-reader, contrast, and small-screen tests.
11. Desktop transfer panel instead of a TUI escape hatch.
12. Hide or label unsupported platform features.

Likely files:

- split Flutter files under `gui/lib/`
- Android Kotlin notification/navigation code
- `src/api.rs`
- profile export/import implementation

## Milestone 8: Test and release engineering

Goal: make every 1.0 claim reproducible.

### Per-PR gate

- `cargo fmt --check`
- `cargo clippy --all-targets -- -D warnings`
- `RUSTFLAGS='-D warnings' cargo check --all-targets`
- `cargo test`
- `flutter analyze`
- `flutter test`
- desktop CLI contract on temporary profile
- migration fixtures
- protocol golden vectors

Add a PR workflow. Do not let mandatory tests silently skip.

### Integration and E2E

1. Replace `test_e2e.sh` with an isolated two-peer embedded-Arti harness.
2. Exchange contacts, send both directions, assert authenticated persistence, restart peers, retry receiver downtime, and transfer a bounded attachment.
3. Add Flutter `integration_test` for Linux and Android with a no-network/fake-engine hook.
4. Add Android lifecycle/process-death tests.
5. Add malformed protocol fuzz/property tests.
6. Add dependency vulnerability and license audit.
7. Run Tor E2E nightly and on release candidates, not every commit.

### Release pipeline

1. Choose and lock Android application ID before public release.
2. Synchronize one version source into Rust, Flutter, Android, AppStream, and artifacts.
3. Pin Rust, Flutter, JDK, NDK, linuxdeploy, and AppImage tools by version/checksum.
4. Release builds fail if production signing credentials are absent. Never publish debug-signed release APKs.
5. Build signed Android AAB/APK and Linux x86_64 AppImage from a clean checkout.
6. Generate SHA-256 manifest, SBOM, provenance, and artifact signatures.
7. Smoke-install/launch artifacts and run temporary-profile contract tests.
8. Publish immutable GitHub Release with changelog and known limitations.
9. Scan artifacts for secrets, profiles, private keys, host paths, and accidental debug material.
10. Add upgrade tests from the previous supported release.

Likely files:

- `.github/workflows/ci.yml`
- `.github/workflows/release.yml`
- `.github/workflows/appimage.yml`
- `run-tests.sh`
- `test_e2e.sh`
- `build.sh`
- `Cargo.toml`
- `gui/pubspec.yaml`
- `gui/android/app/build.gradle.kts`
- `CHANGELOG.md`
- `docs/release-checklist.md`

## Optional pre-1.0 milestone: Bluetooth and local Wi-Fi

Only pull this before 1.0 if Briar-level local transport parity is non-negotiable.

Order:

1. Wi-Fi/LAN first because it can carry complete `WirePacket`s and is easier to test.
2. Bluetooth second because Android permissions, discovery, lifecycle, pairing, MTU, and fragmentation are harder.
3. Keep discovery/address metadata in the adapter. Bind the discovered endpoint to an already known/verified `PeerId` before accepting normal messages.
4. Route immutable encrypted outbox packets over an available endpoint without changing packet encryption or message IDs.
5. Add endpoint preference and simple failover only. Do not build generalized multipath routing yet.
6. Test transport switching, duplicate delivery across Tor/LAN, reordering, loss, and background Android behavior.

Exit criteria:

- Same encrypted packet can be delivered over Tor or local transport.
- Cross-transport duplicates are deduplicated by stable protocol message ID.
- Local discovery cannot silently replace a verified peer endpoint/key.
- Android background behavior is proven on physical devices.

## Post-1.0 provider roadmap

### 1.1: Gmail read-only provider

1. Introduce `AccountProvider` and provider-owned storage separate from Sideband protocol tables.
2. Use OAuth with OS secure credential storage.
3. Implement account lifecycle, token refresh/revoke, incremental sync, pagination, and read-only thread projection.
4. Display Gmail conversations in a unified UI with explicit provider badges and trust semantics.
5. Keep all Gmail bodies, IDs, subjects, and attachments out of Sideband ratchets and transport packets.
6. Add send/reply only after idempotent provider mutation and retry semantics are tested.

Potential later providers can implement the same account connector boundary: generic IMAP/SMTP, Matrix, or others. Each needs a capability model; do not flatten all services into lowest-common-denominator chat.

### 1.x: Wi-Fi and Bluetooth transports

If not included in 1.0, implement after the Tor engine/transport conformance suite is stable. Wi-Fi first, then Bluetooth.

### Later experimental: Meshtastic and MeshCore

Treat each as a separate constrained transport adapter, potentially sharing an internal radio-link helper.

Adapter responsibilities:

- serial/BLE/device lifecycle;
- native address mapping;
- MTU and fragmentation;
- reassembly timeout;
- radio ACK interpretation;
- duty-cycle/backpressure;
- hop limits and duplicate suppression;
- hardware-specific diagnostics.

Initial scope:

- short text and control packets only;
- no attachments;
- no assumption that Tor-sized packets fit;
- capability-driven rejection before send;
- feature-gated and disabled by default;
- hardware-in-loop nightly tests against pinned firmware/device versions.

Do not add Meshtastic/MeshCore fields to Sideband's core packet. If LoRa constraints prove incompatible with the normal packet overhead, define an explicitly reduced low-bandwidth Sideband profile with separate security analysis rather than silently weakening the primary protocol.

### Explicit gateways

If external email or mesh-native messages should later be relayed into Sideband, make that a user-configured gateway:

```text
external event -> consent/policy -> newly authored Sideband message -> Sideband protocol -> selected Sideband transport
```

Show provenance. Never represent an external provider message as end-to-end Sideband traffic when it is not.

## Release candidate definition of done

A 1.0 release candidate is acceptable only when:

- Threat model and protocol/security claims match implementation.
- No unauthenticated packet can mutate trusted state or write normal files/history.
- Session/outbox state survives crash, retry, duplicate, loss, and reorder tests.
- Groups and attachments either meet their security gates or are removed from stable scope.
- Android receives and notifies while the activity is absent, with process-death recovery.
- No normal Android action reaches desktop CLI code or blocks the UI isolate.
- Profile backup/restore and versioned migrations pass upgrade/recovery tests.
- Current two-peer Arti E2E passes in both directions across restart and receiver downtime.
- Per-PR CI and release-candidate gates are mandatory and green.
- Production artifacts are clean, signed, checksummed, smoke-tested, and built from a clean tagged commit.
- Android package identity, signing, minimum API, and supported device matrix are final.
- Linux AppImage support is explicitly scoped to tested architectures/distributions.
- An independent cryptographic/protocol review has no unresolved critical findings.
- README, UI, release notes, and store copy make no claim stronger than the evidence.

## Risks and tradeoffs

- Replacing the ratchet may break current wire/profile compatibility. Pre-1.0 is the least expensive time to do it.
- Keeping groups in 1.0 materially expands the security model and schedule. Direct messaging first is safer.
- Reliable Android background Tor operation may conflict with platform battery/foreground-service restrictions. This needs physical-device evidence, not emulator optimism.
- Supporting Bluetooth/Wi-Fi before 1.0 adds Android permission, lifecycle, discovery, dedupe, and transport-switching complexity. It should not displace protocol hardening.
- Gmail broadens Sideband from a private messenger into a unified communications client. Keep provider identity and storage separate or the security model becomes incoherent.
- LoRa payload and airtime limits may require a reduced protocol profile. File transfer over radio is not a sensible early goal.
- Splitting the monolith is necessary, but a big-bang rewrite would erase working behavior. Extract behind tests incrementally.

## Immediate next implementation sequence

1. Resolve/commit the current unrelated QR-image working tree changes separately.
2. Complete Milestone 0 documents and scope decisions.
3. Write red tests for authentication-before-dispatch and group authorization failures.
4. Add explicit schema/profile version and migration fixtures.
5. Build durable immutable outbox/session transaction semantics.
6. Decide and implement the reviewed session protocol or deliberately static 1.0 crypto.
7. Extract the engine/transport boundary and fake transport conformance tests.
8. Move Android listener ownership out of Flutter activity lifetime and remove synchronous/direct backend drift.
9. Add onboarding, verification, backup/restore, and security UX.
10. Replace stale Tor E2E and make the complete fast gate mandatory in PR CI.
11. Harden and sign the release pipeline.
12. Only then decide whether Bluetooth/Wi-Fi enters 1.0 or 1.1 based on schedule and physical-device evidence.

