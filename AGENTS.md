# AGENTS.md — shared brief for AI assistants

This repository is developed by **ping-ponging between Claude Code and Codex
(GPT)**, usually when one hits a rate limit. This file is the shared source of
truth both assistants read at the **start of every session**. Keep it current:
the incoming assistant trusts this file plus `git log` over re-deriving state.

`CLAUDE.md` just points here. Codex reads this file directly.

---

## What Sideband is

Experimental, privacy-first, serverless chat over Tor onion services (embedded
Arti). Ed25519 identities, X25519 + ChaCha20-Poly1305, Double Ratchet forward
secrecy, SQLite history, file transfer, and local fan-out group chats. **Not
production-secure. Prototype status.**

## The one rule that explains the architecture

**Three thin clients over one Rust core.** Identity, contacts, groups, crypto,
history, and send/receive semantics live in Rust and are never forked per
client.

| Client | How it uses the core |
|---|---|
| TUI/CLI | *is* the core (`sideband tui`, `sideband <subcommand>`) |
| Desktop GUI | Flutter; spawns the `sideband` binary as a subprocess (`_cli` in `main.dart`) |
| Android GUI | Flutter; calls the core over `dart:ffi` into `libsideband.so` (`_MobileApi`) |

So most "cross-client" behavior is core behavior. Fix bugs in the core and all
three benefit; only wire the thin UI shells separately.

## Repo map

- `src/main.rs` — the core: identity, contacts, groups, crypto, DB, `serve()`,
  and the CLI. `lib.rs` `include!`s it, so the same code is both the `sideband`
  binary and the Android cdylib.
- `src/handler.rs` — inbound message handling (`handle_inbound`,
  `handle_text_message`).
- `src/tui.rs` — the ratatui TUI.
- `src/api.rs` — the `#[no_mangle] sideband_api_*` FFI surface the Android app
  calls (lives in the lib crate). JSON-envelope contract: `{"ok":true,"data":…}`
  or `{"ok":false,"error":…}`; every returned string is freed with
  `sideband_api_free_string`.
- `src/interop.rs` — the deterministic cross-peer test harness (no Tor).
- `gui/lib/main.dart` — **both** Flutter GUIs (desktop + Android) in one file.
  `Platform.isAndroid` / `_canUseMobileBackend` switches backend and UI.
- `gui/android/` — manifest, Kotlin `MainActivity` (MethodChannel `sideband/native`),
  `ListenerForegroundService`.
- `build.sh`, `run-tests.sh`, `TESTING.md` — build/test entry points.

## Build

```bash
./build.sh              # all: tui + desktop + android
./build.sh tui          # Rust CLI/TUI        → target/release/sideband
./build.sh desktop      # Linux GUI AppImage  → dist/
./build.sh android      # Rust jniLibs (4 ABIs) + release APK
```
`ANDROID_NDK_HOME` auto-detects from `~/Android/Sdk/ndk/*`.

## Test

```bash
./run-tests.sh fast     # core + interop + FFI + desktop-CLI + Dart. No Tor/emulator. Seconds. THE CI GATE.
./run-tests.sh ui       # + Flutter integration_test (scaffold pending)
./run-tests.sh e2e      # + two-peer over real Tor (slow/flaky; manual)
```
The interop harness (`src/interop.rs`) spins up isolated peers with their own
temp profiles and exchanges real signed+encrypted messages — this is where
contact/group/message-attribution regressions get caught. See `TESTING.md`.

---

## Working economically (model & subagent economy)

Whoever is driving — Claude, Codex, or Hermes — should match the tool to the job.
This is a real cost lever; think about it before defaulting to "top model,
everything inline" or "spawn a subagent for each thing."

- **Tier the model to task difficulty.** Use a cheap/fast model (or fast mode)
  for mechanical work — builds, formatting, running tests, doc edits, routine
  wiring. Reserve the strongest model for hard reasoning — crypto/security
  review, architecture, subtle bugs. Don't run grind on the top model, and don't
  reason about the ratchet on the cheap one.
- **Prefer inline; fan out only when it's genuinely parallel.** Spawning a
  subagent starts it *cold* — it re-derives context the driver already holds — so
  it costs **more** for a single linear task. Reach for subagents only for
  independent, parallel work (e.g. several unrelated reviews or searches at
  once), and give each one a model matched to its slice.
- Mechanisms differ, principle doesn't: Claude Code spawns Task subagents with a
  per-subagent model; Hermes has the subagent-driven-development skill; Codex
  picks one model per session (pick a strong model for reasoning-heavy backlog
  items, a cheaper one for grind).

---

## Handoff protocol (read this before you stop OR start)

**Before you stop** (especially when about to hit a rate limit):
1. Leave the tree green: `./run-tests.sh fast` passes, `cargo clippy --all-targets`
   and `cargo fmt --check` clean, `flutter analyze` clean.
2. Commit in scoped commits with clear messages. **Never `git add .`** — stage
   explicit paths. `fix:`/`feat:`/`test:`/`build:` prefixes.
3. Push (see the git-remote gotcha below).
4. Update the **Current status** section at the bottom of this file.

**When you start:**
1. Read this file, then `git log --oneline -15` and `git status`. The history is
   the state; don't re-derive it.
2. If the task touches the core, remember the client-rebuild gotcha before testing.

---

## Gotchas (hard-won — save the other assistant the pain)

- **Stale binaries bite constantly.** The desktop GUI spawns `sideband` from
  `PATH` (`~/.local/bin/sideband`). After core changes you must
  `./build.sh tui && cp target/release/sideband ~/.local/bin/sideband` or the
  GUI runs old code. Android needs the `.so` rebuilt: `./build.sh android`. The
  TUI header shows its build commit (`@<hash>`) — if it's not a commit in
  `main`, it's stale.
- **Git remote.** `origin`'s default `git@github.com:` resolves (via
  `~/.ssh/config`) to a GitHub account without push access. Push via the
  EvalAlan identity: `git push git@github-evalalan:EvalAlan/sideband.git main`
  (or the remote may already be set to that alias — check `git remote -v`).
- **base64 in `/add`.** Identity keys are STANDARD base64 (`+`, `/`, `=`), not
  URL-safe — never decode them as URL-safe. `/add` parsing must also recover two
  44-char keys that got concatenated when a space was lost copying wrapped
  terminal output (`recover_add_key_fields`).
- **Self is never a contact or group member.** A peer's advertised group-member
  list includes *you*; exclude self by display name (`discover_or_update_group`).
- **No FFI entry point may block on Tor** — it freezes the Flutter UI isolate
  (ANR). Sends enqueue-and-return; delivery status flows via listener events.
- **Profiles are isolated per `--profile` dir** (identity, `contacts.toml`,
  `messages.db`, ratchet state). Tests and harnesses use throwaway temp dirs.
- **Pending contacts can't be replied to** until accepted (TUI `/accept`,
  GUI accept action).

---

## Current status

_Update this section as part of the handoff protocol._

**Shipped:** group chats across all clients; QR share overlay (TUI) + QR scan
(Android); Double-Ratchet "Enable forward secrecy" button; Android foreground
service, message notifications, and mobile-appropriate settings; offline retry
queue; the `src/interop.rs` test harness + FFI/CLI contract tests + tiered
`run-tests.sh`; Linux + Android widget-driven integration tests for first-run,
contact add, `/add`, group creation/sidebar refresh, and group selection;
unified `build.sh`; **Android release-signing scaffolding** (`key.properties`
loading with debug fallback in `build.gradle.kts`, committed
`key.properties.example` template, gitignored secrets, and generation +
verification docs in ANDROID_BUILD.md; signing wiring verified end-to-end with a
throwaway keystore); **encrypted profile export/import** (`sideband export` /
`import`, Argon2id + ChaCha20-Poly1305) for backup and migration, **wired into
both GUIs** (Settings → Export/Import profile: `sideband_api_export_profile` /
`_import_profile` FFI on Android with a `shareFile` intent, CLI on desktop with
native save/open dialogs, passphrase prompt); **Android `applicationId`
migrated** `com.example.sideband_gui` → `com.evalalan.sideband` (Kotlin package
moved too; APK id verified via aapt); **group file sends now file under the
group conversation** on the receiving side and as a single local group record,
instead of showing up as 1:1 PMs (file offer/chunk/inline payloads carry
optional group context; `store_file_message` helper routes them);
**disappearing / expiring messages, end to end** — a signed, sender-enforced
absolute `expires_at_ms` on `ChatMessage` (in `payload_to_sign`, so a relay can't
strip it; backward-compatible via serde default); `expires_at_ms` column on
`messages` + a `conversation_expiry(kind,id,ttl_ms)` default table;
`get/set_conversation_ttl` + `resolve_message_expiry` (per-message override with
tri-state `override_ttl_from_i64`, falling back to the conversation default);
expiry threaded through `send` / `send_retry` / `send_in_conversation` (v2+v3) and
`send_group`; the retry queue carries `expires_at_ms` and drops messages that
expired while queued; `serve` sweeps on its periodic tick (not only on history
load). CLI `send --expire <dur|off>`, `group send --expire`, and an `expiry`
subcommand (`--json` for the GUI). FFI `sideband_api_send_message` takes an
`expires_ms` arg; new `sideband_api_get/set_conversation_expiry`. Both GUIs get a
message-input timer control (per-conversation default + one-shot per-message
override, default OFF). **Android `FLAG_SECURE`** — a "Block screenshots" setting
toggles a `setFlagSecure` MethodChannel (blocks screenshots/recording + recents
preview). Tests: `expiring_message_is_signed_honored_and_swept`,
`send_side_expiry_resolves_default_override_and_survives_retry`,
`enqueue_retry_preserves_absolute_expiry`, plus duration-parse / resolution /
tri-state units.
**Persistent offline outbox** — an undelivered message keeps retrying until
delivered or older than a configurable window (default 24h) instead of being
dropped after 5 attempts. Give-up is age-based in the `serve` loop (`retry_update`
just backs off — 1m/5m/15m then every 30m — and keeps the row); the window lives
in a new `app_settings` key/value table (`get/set_retry_max_age_ms`, generic
`get/set_setting`). Hearing a verified message from a contact wakes all their
queued retries (`retry_wake_contact`). Duplicate-history fix: retries no longer
store a new outbound row per attempt — the original send stores one (Failed) whose
id rides the `retry_queue` (`message_row_id`), and a delivered retry flips it
Failed→Sent (`mark_message_sent`); `store_message_for_conversation[_expiring]` now
return the row id, and the mobile no-listener path stores its row too. CLI
`retry-window [--set <dur>] [--json]`; FFI `sideband_api_get/set_retry_window`;
GUI Settings → "Offline message retry" picker (persisted). Tests:
`retry_update_keeps_row_across_many_failures`, `retry_max_age_defaults_and_persists`,
`mark_message_sent_flips_failed_to_sent`.
**Delivery + read receipts + presence (core; GUI in progress)** — signed+encrypted
`ReceiptPayload` typed message. A receipt references a message by its
`message_replay_fingerprint` (both peers derive it from the ciphertext), so NO new
signed `ChatMessage` field is added (old clients keep verifying). Outbound 1:1 rows
store `msg_fingerprint`; a verified inbound message auto-sends a `delivered` receipt
(fire-and-forget, needs the Tor client threaded through `handle_inbound` →
`handle_text_message(_, tor_client: Option)`). `handle_receipt` verifies then
`mark_delivered_by_fingerprint` (Sent/Failed→Delivered) or `mark_read_up_to`
(contact msgs ≤ up_to_ms → Read). `DeliveryStatus::Read = 3`. Read receipts gated by
a per-profile toggle (`read_receipts_enabled`, default on). Presence is derived: a
returned receipt proves reachability. CLI `read-receipts [--set on|off] [--json]`;
FFI `sideband_api_get/set_read_receipts` + `mark_conversation_read`; serve
`mark_read` control command; `MobileSendPayload::ReadReceipt`. Tests:
`delivery_receipt_marks_sender_message_delivered`,
`read_receipt_marks_messages_read_up_to_timestamp`. Known gap: a message delivered
on a later retry is rebuilt with fresh ciphertext, so its delivered-receipt won't
match the stored fingerprint (stays Sent).

**Open / backlog (roughly prioritized):**
1. `flutter build apk --split-per-abi` option (current APK is a ~134 MB fat APK).
2. Desktop file-transfer UI (currently TUI-only). Light theme.
7. Active presence beacons (vs. the current receipt/activity-derived presence) —
   deferred: periodic per-contact Tor dials are costly and leak a connection pattern.
8. Delivery receipts for retried (offline-then-delivered) messages — see the
   fingerprint gap above; would need the retry to rewrite the row's fingerprint.
3. Group per-message expiry override on the **mobile** FFI (contact overrides work
   on both backends; groups currently honor only the per-conversation default on
   Android — desktop group sends already carry an override via the control channel).
4. `FLAG_SECURE` + notification/tray GUI toggles are still session-scoped. A
   persisted settings store now exists (`app_settings` table + `get/set_setting`,
   used by the retry window) — these toggles could move onto it cheaply.
5. After an in-app import, the running listener still holds the old identity —
   the UI just tells the user to restart. A cleaner flow would reload the profile
   (restart the listener) in place.
6. **Store-and-forward relay/mailbox** (true async delivery even when the sender
   is offline when the recipient returns) — deferred by design; the current outbox
   is sender-held. Needs a relay protocol + a privacy-model discussion (relays
   learn who↔who + timing). See the "next protocol features / Briar" discussion.
