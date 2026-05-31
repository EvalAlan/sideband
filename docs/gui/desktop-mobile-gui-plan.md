# Sideband Desktop + Mobile GUI Plan

Stack choice: **Flutter + flutter_rust_bridge (FRB)**.

Why this default:
- Best mobile support right now (Android/iOS) without fighting webview edge cases.
- Desktop is good enough (Linux/macOS/Windows) from same codebase.
- We keep crypto/Tor/message logic in Rust, UI in Flutter.

## Phase 1 — Rust boundary (do this first)
Goal: stop coupling UI to CLI/TUI internals.

1. Add a stable Rust app API layer (`src/app_api.rs`) with plain request/response types.
2. API methods:
   - `init_profile(profile_path, display_name)`
   - `list_contacts(profile_path)`
   - `add_contact(profile_path, contact)`
   - `delete_contact(profile_path, name)`
   - `send_message(profile_path, to, body)`
   - `send_file(profile_path, to, file_path)`
   - `list_messages(profile_path, contact, limit)`
   - `list_transfers(profile_path)`
   - `resume_transfer(profile_path, hash)`
   - `cancel_transfer(profile_path, hash)`
   - `status(profile_path)`
3. No TUI types in this layer.
4. Keep transport/crypto code where it is for now.

## Phase 2 — FRB bridge
Goal: generated type-safe bindings between Dart and Rust.

1. Add `flutter_rust_bridge` codegen config.
2. Expose Phase-1 API from a Rust lib target.
3. Generate Dart bindings.
4. Add smoke test calling `status()` from Dart.

## Phase 3 — Flutter shell app
Goal: usable MVP UI.

Screens:
1. Profile setup (name + profile path)
2. Contact list + add/delete
3. Chat screen per contact
4. File transfer panel (progress + retry/cancel)
5. Status/debug panel

## Phase 4 — Real-time updates
Goal: chat feels live.

1. Rust event stream (message received, delivery update, transfer update).
2. Dart stream subscription.
3. UI badges/unread counts/connection state.

## Phase 5 — packaging
1. Desktop builds: Linux first, then macOS, Windows.
2. Android build + local install.
3. iOS build after Android path is stable.

## Non-goals (now)
- Fancy animations
- Multi-account UI
- Cloud sync

## Definition of done (MVP)
- Start app on desktop + Android.
- Add contact, send/receive encrypted message.
- Send/receive file with retries.
- Show transfer and connection status.
- No terminal interaction required for normal use.
