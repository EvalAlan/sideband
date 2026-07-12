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
`run-tests.sh`; unified `build.sh`.

**Open / backlog (roughly prioritized):**
1. **UI-driving test tier** — `gui/integration_test/` driven by Flutter's
   `integration_test` (widget finders, not pixels), on Linux and the emulator.
   Needs a "skip the Tor listener on launch" test hook so add-contact and
   message-routing flows run without a 60s bootstrap. This is the gap that let
   the recent Android-only GUI-glue bugs (group tap, `/add`) slip through.
2. Release hygiene for Android: generate a signing keystore
   (`gui/android/key.properties`) and decide on the `applicationId` (changing it
   from `com.example.sideband_gui` orphans existing installs' identities).
3. `flutter build apk --split-per-abi` option (current APK is a ~134 MB fat APK).
4. Desktop file-transfer UI (currently TUI-only). Light theme. Read receipts.
