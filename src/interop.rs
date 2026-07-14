//! Cross-peer interop harness.
//!
//! Spins up isolated peers (each with its own temp profile + identity) and
//! exchanges *real* signed + encrypted messages through the actual
//! build → encode → decode → verify → store pipeline, with **no Tor transport**.
//!
//! This is the deterministic, sub-second test layer that covers the behavior
//! shared by all three clients (TUI, desktop GUI via the CLI, Android via FFI):
//! contact identity, message attribution, auto-discovery of unknown senders,
//! and the double ratchet. Client-specific UI shells are smoke-tested separately.
#![cfg(test)]

use std::path::{Path, PathBuf};

use base64::{engine::general_purpose::STANDARD as B64, Engine};
use tempfile::TempDir;
use tokio::sync::mpsc;

use crate::handler::{handle_receipt, handle_text_message, parse_inbound_line};
use crate::{
    build_outbound_message, contact_add, get_conversation_ttl, init_profile_with_name,
    init_ratchet_for_contact, load_contacts, load_history, load_signing_key, load_x25519_public,
    message_replay_fingerprint, resolve_message_expiry, set_conversation_ttl,
    set_message_fingerprint, store_message_for_conversation, DeliveryStatus, ReceiptPayload,
    TuiEvent,
};

// Valid v3 onion addresses (checksum-correct) for harness peers. They are never
// dialed — the harness moves wire lines by hand — but contact_add validates the
// onion, so they must parse. Generated offline.
const ALICE_ONION: &str = "fpmansl7byak6gq7ymzi7j3dvetjoi6i3oh2yt4tv5y5wgdnn2icuhid.onion";
const BOB_ONION: &str = "qg3dpwh42ldnuy2z42ldce5bc4g6pfpew4s3qti6bngp3hwfrtuvmoqd.onion";

/// One Sideband instance: an isolated profile directory with its own identity.
struct Peer {
    onion: String,
    _dir: TempDir,
    profile: PathBuf,
}

impl Peer {
    fn new(name: &str, onion: &str) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let profile = dir.path().to_path_buf();
        init_profile_with_name(&profile, name).unwrap();
        Peer {
            onion: onion.to_string(),
            _dir: dir,
            profile,
        }
    }

    fn profile(&self) -> &Path {
        &self.profile
    }

    fn ed25519_b64(&self) -> String {
        let key = load_signing_key(&self.profile).unwrap();
        B64.encode(key.verifying_key().to_bytes())
    }

    fn x25519_b64(&self) -> String {
        B64.encode(load_x25519_public(&self.profile).unwrap().as_bytes())
    }

    /// Add `other` to this peer's contacts under the name `as_name`.
    fn add_contact(&self, other: &Peer, as_name: &str) {
        contact_add(
            &self.profile,
            as_name,
            &other.onion,
            &other.ed25519_b64(),
            &other.x25519_b64(),
        )
        .unwrap();
    }

    /// Build the wire line for a text message from this peer to the contact
    /// named `to_name` (must already be one of this peer's contacts).
    fn make_message(&self, to_name: &str, text: &str) -> String {
        let msg =
            build_outbound_message(&self.profile, to_name, "msg", text, &self.onion, None).unwrap();
        serde_json::to_string(&msg).unwrap()
    }
}

/// Deliver a wire `line` into `to`'s inbound pipeline (decrypt → verify →
/// attribute → store) and return the `InboundMessage` events the UI would
/// receive, as `(contact, body, verified)` tuples.
async fn deliver(to: &Peer, line: &str) -> Vec<(String, String, bool)> {
    let contacts = load_contacts(to.profile()).unwrap();
    let mut msg = parse_inbound_line(line)
        .unwrap()
        .expect("wire line must parse into a ChatMessage");
    let (tx, mut rx) = mpsc::channel::<TuiEvent>(64);
    // No Tor client in the harness → no auto delivery receipt is sent.
    handle_text_message(to.profile(), &tx, &contacts, &mut msg, None)
        .await
        .unwrap();
    drop(tx);
    let mut out = Vec::new();
    while let Ok(ev) = rx.try_recv() {
        if let TuiEvent::InboundMessage {
            contact,
            body,
            verified,
            ..
        } = ev
        {
            out.push((contact, body, verified));
        }
    }
    out
}

/// Issue 3: a message from a peer that is ALREADY a known (non-pending) contact
/// must be attributed to that contact, verify, and be stored where the UI looks
/// for it — not dropped or filed under a raw pubkey / "verified-peer".
#[tokio::test]
async fn message_from_known_contact_shows_under_that_contact() {
    let alice = Peer::new("alice", ALICE_ONION);
    let bob = Peer::new("bob", BOB_ONION);
    alice.add_contact(&bob, "bob");
    bob.add_contact(&alice, "alice"); // bob already has alice (e.g. added on his phone)

    let line = alice.make_message("bob", "hello bob");
    let events = deliver(&bob, &line).await;

    assert_eq!(events.len(), 1, "exactly one inbound event expected");
    assert_eq!(
        events[0].0, "alice",
        "message must be attributed to the existing contact 'alice'"
    );
    assert_eq!(events[0].1, "hello bob");
    assert!(events[0].2, "message from a known contact must verify");

    let hist = load_history(bob.profile(), Some("alice"), 50).unwrap();
    assert!(
        hist.iter().any(|r| r.body == "hello bob"),
        "message must be stored under 'alice' so the UI can display it"
    );
}

/// A message from a genuinely unknown sender must still surface and be preserved
/// (trust-on-first-contact), not silently dropped.
#[tokio::test]
async fn message_from_unknown_sender_is_preserved() {
    let alice = Peer::new("alice", ALICE_ONION);
    let bob = Peer::new("bob", BOB_ONION);
    alice.add_contact(&bob, "bob");
    // bob does NOT know alice yet.

    let line = alice.make_message("bob", "who is this");
    let events = deliver(&bob, &line).await;

    assert_eq!(events.len(), 1, "unknown-sender message must still surface");
    assert_eq!(events[0].1, "who is this");

    let all = load_history(bob.profile(), None, 50).unwrap();
    assert!(
        all.iter().any(|r| r.body == "who is this"),
        "first message from an unknown sender must be preserved in history"
    );
}

/// Issue 2 regression: base64 keys containing `+`, `/`, and `=` (STANDARD
/// base64, not URL-safe) must validate and round-trip through `contact_add`.
/// These are a real peer's `/share` keys. Guards against a URL-safe-decode
/// regression that would reject roughly half of all real identities.
#[test]
fn contact_add_accepts_standard_base64_keys() {
    use crate::validate_contact_fields;
    const ONION: &str = "qdnx34k2b3fzp3umv7ryvzxtbzjluzkvuvqixvuooy43b5n6lddaspid.onion";
    const ED: &str = "fLo7TRtqCxE2wtjTvNvUJjRDBewhYV7bkW3P/F/451w=";
    const X: &str = "K4+eWfSYw8TtmsViirLxsNs7zAWzKQ/YtJtQFVcncUk=";
    assert!(ED.contains('/') && X.contains('+') && X.ends_with('='));
    validate_contact_fields(ONION, ED, X).expect("standard-base64 keys must validate");

    let dir = tempfile::tempdir().unwrap();
    init_profile_with_name(dir.path(), "me").unwrap();
    contact_add(dir.path(), "Rocky", ONION, ED, X).unwrap();
    let contacts = load_contacts(dir.path()).unwrap();
    assert_eq!(contacts.get("Rocky").unwrap().pubkey_b64, ED);
}

/// Issue 2 (real cause): a `/add` line whose two 44-char base64 keys got
/// concatenated (space lost copying wrapped terminal output) must be recovered
/// into two separate keys so the contact still adds.
#[test]
fn recover_add_key_fields_splits_concatenated_keys() {
    let ed = "fLo7TRtqCxE2wtjTvNvUJjRDBewhYV7bkW3P/F/451w=";
    let x = "K4+eWfSYw8TtmsViirLxsNs7zAWzKQ/YtJtQFVcncUk=";
    let concatenated = format!("{ed}{x}"); // 88 chars, no space
    let fields = crate::recover_add_key_fields(vec![
        "Rocky".to_string(),
        "example.onion".to_string(),
        concatenated,
    ]);
    assert_eq!(
        fields.len(),
        4,
        "concatenated keys must be split back into two"
    );
    assert_eq!(fields[2], ed);
    assert_eq!(fields[3], x);

    // A well-formed 4-field line is left untouched.
    let ok = crate::recover_add_key_fields(vec![
        "Rocky".to_string(),
        "example.onion".to_string(),
        ed.to_string(),
        x.to_string(),
    ]);
    assert_eq!(ok.len(), 4);
    assert_eq!(ok[2], ed);
}

/// A passphrase-encrypted profile export must round-trip: identity, contacts,
/// display name, and ratchet state restore into a fresh profile, and the wrong
/// passphrase must fail.
#[test]
fn profile_export_import_round_trips() {
    const ONION: &str = "qdnx34k2b3fzp3umv7ryvzxtbzjluzkvuvqixvuooy43b5n6lddaspid.onion";
    const ED: &str = "fLo7TRtqCxE2wtjTvNvUJjRDBewhYV7bkW3P/F/451w=";
    const X: &str = "K4+eWfSYw8TtmsViirLxsNs7zAWzKQ/YtJtQFVcncUk=";

    let src = tempfile::tempdir().unwrap();
    init_profile_with_name(src.path(), "Mercury").unwrap();
    contact_add(src.path(), "Rocky", ONION, ED, X).unwrap();
    init_ratchet_for_contact(src.path(), "Rocky").unwrap();

    let archive = crate::export_profile_bytes(src.path(), "hunter2").unwrap();
    assert!(archive.starts_with(b"SBEXP1\n"));

    // Wrong passphrase must not decrypt.
    let wrong = tempfile::tempdir().unwrap();
    assert!(crate::import_profile_bytes(wrong.path(), &archive, "nope", false).is_err());

    // Correct passphrase restores into a fresh profile.
    let dst = tempfile::tempdir().unwrap();
    crate::import_profile_bytes(dst.path(), &archive, "hunter2", false).unwrap();

    assert_eq!(
        std::fs::read_to_string(src.path().join("identity.toml")).unwrap(),
        std::fs::read_to_string(dst.path().join("identity.toml")).unwrap(),
        "identity must be identical after import"
    );
    assert_eq!(crate::load_display_name(dst.path()).unwrap(), "Mercury");
    assert!(load_contacts(dst.path()).unwrap().contains_key("Rocky"));
    assert!(
        dst.path().join("ratchet").join("Rocky.bin").exists(),
        "ratchet state must be restored"
    );

    // Importing over an existing identity is refused without overwrite.
    assert!(crate::import_profile_bytes(dst.path(), &archive, "hunter2", false).is_err());
    crate::import_profile_bytes(dst.path(), &archive, "hunter2", true).unwrap();
}

/// A sender-set message expiry must (a) still verify — proving it's inside the
/// signed payload, not strippable — (b) be honored on the receiving side, and
/// (c) get swept once past, while a not-yet-expired message survives.
#[tokio::test]
async fn expiring_message_is_signed_honored_and_swept() {
    let alice = Peer::new("alice", ALICE_ONION);
    let bob = Peer::new("bob", BOB_ONION);
    alice.add_contact(&bob, "bob");
    bob.add_contact(&alice, "alice");

    // Already-expired (past absolute expiry).
    let expired = build_outbound_message(
        alice.profile(),
        "bob",
        "msg",
        "boom",
        &alice.onion,
        Some(1_000),
    )
    .unwrap();
    let events = deliver(&bob, &serde_json::to_string(&expired).unwrap()).await;
    assert!(
        events[0].2,
        "an expiring message must still verify (expiry is signed, not strippable)"
    );
    assert!(
        load_history(bob.profile(), Some("alice"), 50)
            .unwrap()
            .iter()
            .all(|r| r.body != "boom"),
        "an already-expired message must be swept from history"
    );

    // Future expiry survives.
    let future = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis()
        + 3_600_000;
    let live = build_outbound_message(
        alice.profile(),
        "bob",
        "msg",
        "keep me",
        &alice.onion,
        Some(future),
    )
    .unwrap();
    deliver(&bob, &serde_json::to_string(&live).unwrap()).await;
    assert!(
        load_history(bob.profile(), Some("alice"), 50)
            .unwrap()
            .iter()
            .any(|r| r.body == "keep me"),
        "a not-yet-expired message must remain"
    );
}

/// The send-side resolution wiring: a per-conversation default TTL produces an
/// absolute expiry on a plain send, a per-message override beats the default
/// (including an explicit OFF), and a queued retry preserves the expiry.
#[tokio::test]
async fn send_side_expiry_resolves_default_override_and_survives_retry() {
    let alice = Peer::new("alice", ALICE_ONION);
    let bob = Peer::new("bob", BOB_ONION);
    alice.add_contact(&bob, "bob");
    bob.add_contact(&alice, "alice");

    let now = || {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis()
    };

    // No default set: a plain send has no expiry.
    assert_eq!(
        resolve_message_expiry(alice.profile(), "contact", "bob", None).unwrap(),
        None
    );

    // Set a 1h per-conversation default; a plain send now expires ~1h out.
    set_conversation_ttl(alice.profile(), "contact", "bob", Some(3_600_000)).unwrap();
    assert_eq!(
        get_conversation_ttl(alice.profile(), "contact", "bob").unwrap(),
        Some(3_600_000)
    );
    let resolved = resolve_message_expiry(alice.profile(), "contact", "bob", None)
        .unwrap()
        .expect("default should produce an expiry");
    assert!((resolved as i128 - (now() + 3_600_000) as i128).abs() < 5_000);

    // A per-message OFF override wins over the default.
    assert_eq!(
        resolve_message_expiry(alice.profile(), "contact", "bob", Some(None)).unwrap(),
        None
    );
    // A per-message TTL override wins over the default.
    let overridden = resolve_message_expiry(alice.profile(), "contact", "bob", Some(Some(60_000)))
        .unwrap()
        .expect("override should produce an expiry");
    assert!((overridden as i128 - (now() + 60_000) as i128).abs() < 5_000);

    // End to end: a resolved expiry rides the signed wire message and lands with
    // the same deadline on the receiver.
    let msg = build_outbound_message(
        alice.profile(),
        "bob",
        "msg",
        "vanishing",
        &alice.onion,
        Some(resolved),
    )
    .unwrap();
    let events = deliver(&bob, &serde_json::to_string(&msg).unwrap()).await;
    assert!(events[0].2, "expiring message must verify");
    // The message is present now (expiry is in the future) on both ends.
    assert!(load_history(bob.profile(), Some("alice"), 50)
        .unwrap()
        .iter()
        .any(|r| r.body == "vanishing"));
}

/// Alice sends a message; Bob's node returns a delivery receipt. Alice's outbound
/// row must flip Sent → Delivered, matched by the ciphertext fingerprint both
/// peers derive independently.
#[tokio::test]
async fn delivery_receipt_marks_sender_message_delivered() {
    let alice = Peer::new("alice", ALICE_ONION);
    let bob = Peer::new("bob", BOB_ONION);
    alice.add_contact(&bob, "bob");
    bob.add_contact(&alice, "alice");

    // Alice builds + stores an outbound message to Bob (as a real send would).
    let sent = build_outbound_message(alice.profile(), "bob", "msg", "hi bob", &alice.onion, None)
        .unwrap();
    let fp = message_replay_fingerprint(&sent);
    let row = store_message_for_conversation(
        alice.profile(),
        "out",
        "bob",
        BOB_ONION,
        "hi bob",
        sent.timestamp_ms,
        DeliveryStatus::Sent,
        "contact",
        "bob",
    )
    .unwrap();
    set_message_fingerprint(alice.profile(), row, &fp).unwrap();

    // Bob's node acknowledges delivery (fingerprint is identical on both ends).
    let payload = ReceiptPayload {
        kind: "receipt".into(),
        state: "delivered".into(),
        msg_fingerprint: Some(fp),
        up_to_ms: None,
    };
    let mut receipt = build_outbound_message(
        bob.profile(),
        "alice",
        "receipt",
        &serde_json::to_string(&payload).unwrap(),
        &bob.onion,
        None,
    )
    .unwrap();
    let contacts = load_contacts(alice.profile()).unwrap();
    handle_receipt(alice.profile(), &contacts, &mut receipt)
        .await
        .unwrap();

    let hist = load_history(alice.profile(), Some("bob"), 50).unwrap();
    let m = hist.iter().find(|m| m.body == "hi bob").unwrap();
    assert_eq!(
        m.status,
        DeliveryStatus::Delivered as i64,
        "a delivery receipt must mark the sender's message delivered"
    );
}

/// A read receipt acknowledges every message up to a timestamp: Alice's messages
/// at or before `up_to_ms` flip to Read, later ones stay behind.
#[tokio::test]
async fn read_receipt_marks_messages_read_up_to_timestamp() {
    let alice = Peer::new("alice", ALICE_ONION);
    let bob = Peer::new("bob", BOB_ONION);
    alice.add_contact(&bob, "bob");
    bob.add_contact(&alice, "alice");

    let store = |body: &str, ts: u128| {
        store_message_for_conversation(
            alice.profile(),
            "out",
            "bob",
            BOB_ONION,
            body,
            ts,
            DeliveryStatus::Sent,
            "contact",
            "bob",
        )
        .unwrap()
    };
    store("first", 1_000);
    store("second", 2_000);
    store("third", 3_000);

    let payload = ReceiptPayload {
        kind: "receipt".into(),
        state: "read".into(),
        msg_fingerprint: None,
        up_to_ms: Some(2_000),
    };
    let mut receipt = build_outbound_message(
        bob.profile(),
        "alice",
        "receipt",
        &serde_json::to_string(&payload).unwrap(),
        &bob.onion,
        None,
    )
    .unwrap();
    let contacts = load_contacts(alice.profile()).unwrap();
    handle_receipt(alice.profile(), &contacts, &mut receipt)
        .await
        .unwrap();

    let hist = load_history(alice.profile(), Some("bob"), 50).unwrap();
    let status = |body: &str| hist.iter().find(|m| m.body == body).unwrap().status;
    assert_eq!(status("first"), DeliveryStatus::Read as i64);
    assert_eq!(status("second"), DeliveryStatus::Read as i64);
    assert_eq!(
        status("third"),
        DeliveryStatus::Sent as i64,
        "a message sent after the read watermark must stay unread"
    );
}

/// A file sent to a group must be filed under the group conversation on the
/// receiving side, not as a 1:1 PM (regression: group files showed up as PMs).
#[tokio::test]
async fn group_inline_file_lands_in_group_not_pm() {
    use sha2::Digest;

    let alice = Peer::new("alice", ALICE_ONION);
    let bob = Peer::new("bob", BOB_ONION);
    alice.add_contact(&bob, "bob");
    bob.add_contact(&alice, "alice");

    // A file_inline payload tagged with a group (what send_file_to_group emits).
    let data = b"hello group file";
    let hash = {
        let mut h = sha2::Sha256::new();
        h.update(data);
        format!("{:x}", h.finalize())
    };
    let inline_json = serde_json::json!({
        "name": "note.txt",
        "size": data.len(),
        "hash": hash,
        "data_b64": B64.encode(data),
        "group_id": "grp1",
        "group_title": "Homies",
    })
    .to_string();
    let msg = build_outbound_message(
        alice.profile(),
        "bob",
        "file_inline",
        &inline_json,
        "",
        None,
    )
    .unwrap();
    let line = serde_json::to_string(&msg).unwrap();

    let contacts = load_contacts(bob.profile()).unwrap();
    let mut parsed = parse_inbound_line(&line).unwrap().unwrap();
    let (tx, mut rx) = mpsc::channel::<TuiEvent>(16);
    crate::handler::handle_file_inline(bob.profile(), &tx, &contacts, &mut parsed)
        .await
        .unwrap();
    drop(tx);

    // Stored under the group conversation, not a contact PM.
    let all = load_history(bob.profile(), None, 50).unwrap();
    let row = all
        .iter()
        .find(|r| r.body.contains("[file received:"))
        .expect("received file must be stored");
    assert_eq!(row.conversation_kind, "group");
    assert_eq!(row.conversation_id, "grp1");
    assert!(
        load_history(bob.profile(), Some("alice"), 50)
            .unwrap()
            .iter()
            .all(|r| !r.body.contains("[file received:")),
        "the file must NOT appear as a PM under 'alice'"
    );

    // And a group event (not a plain InboundMessage) was emitted.
    let mut got_group_event = false;
    while let Ok(ev) = rx.try_recv() {
        if let TuiEvent::InboundGroupMessage { group_id, .. } = ev {
            assert_eq!(group_id, "grp1");
            got_group_event = true;
        }
    }
    assert!(got_group_event, "expected an InboundGroupMessage event");
}

/// Discovering a group from a peer must never add our own identity as a contact
/// or a group member — the UI represents self implicitly as "You".
#[test]
fn discover_group_excludes_self() {
    let dir = tempfile::tempdir().unwrap();
    init_profile_with_name(dir.path(), "Mercury").unwrap();

    // "Rocky" sends a group whose advertised members include us (Mercury).
    let group = crate::discover_or_update_group(
        dir.path(),
        "grp1",
        "Homies",
        "Rocky",
        &["Alan".to_string(), "Mercury".to_string()],
    )
    .unwrap();

    assert!(
        group.members.iter().all(|m| m.contact != "Mercury"),
        "self must not be listed as a group member"
    );
    let contacts = load_contacts(dir.path()).unwrap();
    assert!(
        !contacts.contains_key("Mercury"),
        "self must not be added to contacts"
    );
    assert!(
        contacts.contains_key("Rocky"),
        "the sender is a real member"
    );
    assert!(contacts.contains_key("Alan"), "other members are added");
}

/// A v3 double-ratchet message (after /ratchet) must decrypt and verify on the
/// receiving side.
#[tokio::test]
async fn ratchet_message_roundtrips() {
    let alice = Peer::new("alice", ALICE_ONION);
    let bob = Peer::new("bob", BOB_ONION);
    alice.add_contact(&bob, "bob");
    bob.add_contact(&alice, "alice");

    init_ratchet_for_contact(alice.profile(), "bob").unwrap();

    let line = alice.make_message("bob", "ratchet hi");
    let events = deliver(&bob, &line).await;

    assert_eq!(events.len(), 1);
    assert_eq!(events[0].1, "ratchet hi");
    assert!(events[0].2, "v3 ratchet message must verify and decrypt");
}
