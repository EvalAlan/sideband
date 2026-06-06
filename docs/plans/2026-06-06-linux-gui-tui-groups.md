# Linux GUI/TUI Group Chats Implementation Plan

> **For Hermes:** Use subagent-driven-development skill to implement this plan task-by-task.

**Goal:** Make Linux GUI/TUI the primary proving ground, harden AppImage packaging, then add group chats without painting mobile into a corner.

**Architecture:** Keep transport peer-to-peer over existing Tor contacts. A group is local metadata plus fan-out delivery to member contacts, with a group-scoped conversation ID in persisted history. Inbound group messages are normal encrypted peer messages containing a signed group payload; every recipient verifies sender membership locally before storing under the group conversation.

**Tech Stack:** Rust CLI/TUI, SQLite via rusqlite, JSON message payloads via serde, Flutter Linux GUI, linuxdeploy AppImage.

---

## Non-negotiables

- Do not create a central group server. That would be Sideband cosplay with Slack's guts.
- Do not fork the message stack for GUI/TUI/mobile. Rust owns identity, contacts, groups, share payloads, history, and send semantics.
- GUI sends keep using the `sideband serve` stdin control channel.
- Group delivery starts as deterministic fan-out. Later ACK/read-state can be layered on top.
- Store enough metadata now to support mobile later: stable group ID, display title, member identity keys, member roles, created/updated timestamps.

---

## Phase 0: AppImage packaging hardening

### Task 0.1: Fix AppImage wrapper recursion

**Objective:** Ensure the AppImage wrapper launches the real Flutter binary instead of overwriting and recursively executing itself.

**Files:**
- Modify: `gui/build-appimage.sh`

**Steps:**
1. Move the Flutter bundle binary from `sideband_gui` to `sideband_gui.bin` after copying the release bundle into `AppDir/usr/bin/`.
2. Write the wrapper as `AppDir/usr/bin/sideband_gui`.
3. Make the wrapper exec `sideband_gui.bin`.
4. Run: `bash -n gui/build-appimage.sh`.
5. Commit: `git commit -m "fix: harden appimage build wrapper"`.

### Task 0.2: Make AppImage tooling architecture-aware

**Objective:** Support x86_64 Linux dev machines and avoid hardcoded x64 paths that break on arm64 builders.

**Files:**
- Modify: `gui/build-appimage.sh`

**Steps:**
1. Derive Flutter bundle arch from `uname -m`: `x86_64 -> x64`, `aarch64 -> arm64`.
2. Derive linuxdeploy AppImage arch from `uname -m`: `x86_64`, `aarch64`.
3. Download `linuxdeploy-${APPIMAGE_ARCH}.AppImage` and matching Flutter plugin.
4. Symlink the plugin to `~/.local/bin/linuxdeploy-plugin-flutter` and prepend `~/.local/bin` to `PATH`.
5. Use `APPIMAGE_EXTRACT_AND_RUN=1` instead of passing `--appimage-extract-and-run` as a linuxdeploy argument.
6. Run: `bash -n gui/build-appimage.sh`.

### Task 0.3: Build and smoke-test AppImage on mercury

**Objective:** Prove the packaging flow works on Alan's Linux desktop.

**Files:**
- Read only unless failures appear: `gui/build-appimage.sh`, `gui/linux/sideband_gui.desktop`

**Steps:**
1. Run from repo root: `cd ~/repos/sideband && ./gui/build-appimage.sh`.
2. Verify `dist/*.AppImage` exists.
3. Run the AppImage with an isolated profile: `SIDEBAND_PROFILE=/tmp/sideband-appimage-profile ./dist/*.AppImage`.
4. Verify the GUI opens, initializes a profile, and starts listener bootstrap.
5. If runtime libraries are missing, patch the AppDir generation, not the user shell.

---

## Phase 1: Group data model

### Task 1.1: Add group structs and persistence tables

**Objective:** Persist group definitions independently of UI.

**Files:**
- Modify: `src/main.rs`
- Test: existing `#[cfg(test)] mod tests` in `src/main.rs`

**Schema:**
```sql
CREATE TABLE IF NOT EXISTS groups (
    id          TEXT PRIMARY KEY,
    title       TEXT NOT NULL,
    created_at_ms INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS group_members (
    group_id    TEXT NOT NULL,
    contact     TEXT NOT NULL,
    role        TEXT NOT NULL DEFAULT 'member',
    added_at_ms INTEGER NOT NULL,
    PRIMARY KEY (group_id, contact),
    FOREIGN KEY (group_id) REFERENCES groups(id) ON DELETE CASCADE
);
```

**Steps:**
1. Add `GroupFile`/`GroupMember`-equivalent Rust structs or DB row structs.
2. Extend `init_db()` with `groups` and `group_members` tables.
3. Add `create_group(profile, title, members) -> Result<GroupInfo>`.
4. Add `load_groups(profile) -> Result<Vec<GroupInfo>>`.
5. Add test: create temp profile, create group with two contacts, reload groups, assert ID/title/members.
6. Run: `cargo test --quiet db_group_create_and_load`.

### Task 1.2: Add conversation kind to history without breaking old rows

**Objective:** Let GUI/TUI load peer conversations and group conversations from the same history command.

**Files:**
- Modify: `src/main.rs`

**Schema migration:**
```sql
ALTER TABLE messages ADD COLUMN conversation_kind TEXT NOT NULL DEFAULT 'contact';
ALTER TABLE messages ADD COLUMN conversation_id TEXT NOT NULL DEFAULT '';
```

**Steps:**
1. Add migration helper that checks `PRAGMA table_info(messages)` before ALTER. SQLite doesn't do `ADD COLUMN IF NOT EXISTS`; don't pretend it does.
2. Update `store_message()` to accept conversation kind/id, or add `store_group_message()` if changing all call sites is too noisy.
3. Existing peer messages must still store/load as `conversation_kind='contact'` and `conversation_id=contact`.
4. Add test loading old-style rows through new history path.
5. Run: `cargo test --quiet db_store_and_load_history`.

---

## Phase 2: CLI group commands

### Task 2.1: Add `sideband group create/list/show/delete`

**Objective:** Make groups usable before touching UI.

**Files:**
- Modify: `src/main.rs`

**Commands:**
```bash
sideband group create --title "Ops" --member alice --member bob [--json]
sideband group list [--json]
sideband group show --id <group-id> [--json]
sideband group delete --id <group-id>
```

**Steps:**
1. Add `Group` subcommand and `GroupAction` enum.
2. Implement create/list/show/delete through Rust persistence helpers.
3. Generate group IDs as stable random URL-safe tokens or BLAKE3/sha256-derived IDs from creator pubkey + random nonce. Do not use title as ID. People rename things. Constantly.
4. Add CLI smoke tests if the current test structure supports invoking command helpers; otherwise add function-level tests.
5. Run: `cargo test --quiet`.

### Task 2.2: Add `sideband group member add/remove`

**Objective:** Manage group membership from one canonical backend path.

**Commands:**
```bash
sideband group member add --id <group-id> --contact alice
sideband group member remove --id <group-id> --contact alice
```

**Steps:**
1. Implement DB upsert/delete for group members.
2. Refuse to add unknown contacts.
3. Refuse to remove the last member unless `--force` exists and is explicitly passed.
4. Add tests for unknown member and duplicate add.
5. Run: `cargo test --quiet group_member`.

---

## Phase 3: Group message wire format and fan-out

### Task 3.1: Define group payload

**Objective:** Keep group metadata inside encrypted peer messages while preserving existing E2E contact encryption.

**Payload:**
```json
{
  "kind": "group_message",
  "group_id": "...",
  "group_title": "Ops",
  "sender_contact_hint": "alice",
  "body": "hello group",
  "client_msg_id": "..."
}
```

**Steps:**
1. Add `GroupMessagePayload` struct.
2. Send it as `ChatMessage.type = "group_message"` or as typed plaintext under existing v2/v3 encryption.
3. Inbound handler decrypts first, parses payload second, verifies sender key maps to a member of the group.
4. Store under `conversation_kind='group'`, `conversation_id=group_id`, contact/sender as the sender contact.
5. Add tests that non-member group messages store as failed/system warning or are rejected.

### Task 3.2: Add group fan-out send helper

**Objective:** Send one group message to each member contact using the existing contact send path and send mutex.

**Files:**
- Modify: `src/main.rs`

**Steps:**
1. Add `send_group(profile, tor_client, group_id, body)` helper.
2. Load group members.
3. For each member, call existing typed send path with group payload.
4. Store one local outbound group message after fan-out begins, with aggregate status: sent if at least one member succeeds, failed if none succeeds.
5. Return per-member delivery results as JSON for GUI/TUI status display.
6. Add test for fan-out target resolution without Tor by extracting target list logic into a pure function.

### Task 3.3: Extend serve control channel

**Objective:** Let GUI send group messages without spawning a second Tor client.

**Control command:**
```json
{"cmd":"send_group","group_id":"...","message":"hello"}
```

**Steps:**
1. Extend control command enum/parser.
2. Route `send_group` through the same send mutex used by contact sends.
3. Emit clear stdout lines: `group message sent group=<id> ok=<n> failed=<n>`.
4. Emit errors as `group send error: ...`.
5. Add parser/unit tests where possible.

---

## Phase 4: TUI support

### Task 4.1: Merge contacts and groups into conversation list

**Objective:** Let TUI select peer chats and group chats in the same left pane.

**Files:**
- Modify: `src/tui.rs`

**Steps:**
1. Replace `contacts: Vec<String>` with `conversations: Vec<ConversationRef>`.
2. `ConversationRef` has `{kind, id, title}`.
3. Render contacts normally and groups with a small prefix like `# Ops`.
4. Load history with conversation kind/id for selected item.
5. Run: `cargo test --quiet` and manual `cargo run -- tui --profile /tmp/sideband-tui-a`.

### Task 4.2: Add TUI slash commands

**Commands:**
```text
/group create <title> <member> [member...]
/group list
/group show <group-id>
/group add <group-id> <contact>
/group remove <group-id> <contact>
/group delete <group-id>
```

**Steps:**
1. Parse slash commands before requiring selected contact.
2. Refresh conversation list after mutations.
3. Plain Enter sends to selected contact or selected group based on selected conversation kind.
4. Update `/help`.
5. Manual test with two local profiles once Tor bootstrap is available.

---

## Phase 5: Flutter GUI support

### Task 5.1: Represent group conversations in Dart

**Objective:** Prepare the existing single-file GUI for mixed peer/group conversations.

**Files:**
- Modify: `gui/lib/main.dart`

**Steps:**
1. Replace contact-only UI state with `Conversation` model: kind, id, title, subtitle.
2. Load contact conversations from `sideband contact list --json`.
3. Load group conversations from `sideband group list --json`.
4. Keep selected conversation stable across refreshes.
5. History loads by kind/id, not just contact name.

### Task 5.2: Add group management dialogs

**Objective:** Make groups usable without slash-command archaeology.

**UI:**
- New group button.
- Group details dialog.
- Add/remove member actions.
- Delete group confirmation.

**Steps:**
1. Use CLI JSON commands for group create/list/show/member mutation.
2. Do not duplicate DB logic in Dart.
3. Keep outputs selectable for debugging.
4. Refresh conversations after mutation.
5. Format: `gui/.tools/flutter/bin/dart format gui/lib/main.dart`.

### Task 5.3: Send group messages over listener control channel

**Objective:** Keep GUI group sends on the correctness path.

**Steps:**
1. If selected conversation is contact, keep existing send command.
2. If selected conversation is group, write `send_group` JSON to serve stdin.
3. Add optimistic local group message and dedupe against group history.
4. Suppress transient Tor churn in the banner using existing recent-send logic, extended for `group send error`.
5. Run: `gui/.tools/flutter/bin/flutter analyze` if available.

---

## Phase 6: Runtime acceptance

### Task 6.1: Two-peer peer-chat regression

**Objective:** Prove existing one-to-one messaging still works.

**Steps:**
1. Create `/tmp/sideband-a` and `/tmp/sideband-b` profiles.
2. Start two listeners.
3. Exchange share commands.
4. Send A -> B and B -> A.
5. Verify history on both profiles.

### Task 6.2: Three-peer group smoke test

**Objective:** Prove group fan-out works over independent processes.

**Steps:**
1. Create profiles A/B/C.
2. Exchange contacts among all three.
3. On A: create group with B and C.
4. Send A -> group.
5. Verify B and C store inbound under the same group ID.
6. Send B -> group if B has group metadata. If group metadata propagation is not implemented yet, document that limitation explicitly.

---

## Staging and commit discipline

- Run `git status --short` before each commit.
- Stage only intended files with explicit paths.
- Re-check staged diff: `git diff --cached --name-only`.
- Use scoped commits: packaging, group data model, CLI groups, TUI groups, GUI groups.
- Never `git add .`. That's how unrelated garbage gets shipped.
