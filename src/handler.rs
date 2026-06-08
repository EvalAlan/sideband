//! Inbound message handling.
//!
//! `handle_inbound` takes a raw line from the transport, parses it into a
//! [`ChatMessage`], decrypts/verifies it, dispatches on message-type, persists
//! to SQLite, and emits [`TuiEvent`] notifications.

use std::path::Path;
use std::sync::Arc;

use anyhow::Result;
use base64::{engine::general_purpose::STANDARD as B64, Engine};
use tokio::sync::mpsc;

use crate::transport::tor::TorTransport;
use crate::{
    decrypt_and_verify, discover_or_update_group, resolve_contact_name_by_pubkey,
    send_typed_message, store_message, store_message_for_conversation, ChatMessage, ContactsMap,
    DeliveryStatus, FileAckPayload, FileChunkPayload, FileInlinePayload, FileOfferPayload,
    GroupMessagePayload, IncomingFileState, TuiEvent,
};

/// Parse a raw inbound line into a [`ChatMessage`].
///
/// Returns `Ok(None)` if the line is empty or not valid JSON.
/// Returns `Ok(Some(msg))` on success.
pub fn parse_inbound_line(line: &str) -> Result<Option<ChatMessage>> {
    let envelope = TorTransport::raw_line_to_envelope(line.trim());
    let raw = match TorTransport::envelope_body_as_str(&envelope) {
        Ok(r) => r,
        Err(_) => return Ok(None),
    };
    match serde_json::from_str::<ChatMessage>(raw) {
        Ok(msg) => Ok(Some(msg)),
        Err(_) => Ok(None),
    }
}

/// Handle a single inbound [`ChatMessage`].
///
/// This function:
///  1. Decrypts and verifies the message signature.
///  2. Dispatches on `msg.type` (normal / file_offer / file_chunk / file_ack).
///  3. Persists to SQLite via [`store_message`].
///  4. Emits a [`TuiEvent::InboundMessage`] for the UI.
///  5. For file chunks: sends ACK, reassembles, verifies hash, writes to
///     `~/.sideband/downloads/`.
pub async fn handle_inbound(
    profile: &Path,
    tui_tx: &mpsc::Sender<TuiEvent>,
    contacts: &ContactsMap,
    msg: &mut ChatMessage,
    tor_client: Arc<arti_client::TorClient<tor_rtcompat::PreferredRuntime>>,
) -> Result<()> {
    if msg.r#type == "file_offer" {
        handle_file_offer(profile, tui_tx, contacts, msg).await?;
        return Ok(());
    }

    if msg.r#type == "file_chunk" {
        handle_file_chunk(profile, tui_tx, contacts, msg, tor_client).await?;
        return Ok(());
    }

    if msg.r#type == "file_inline" {
        handle_file_inline(profile, tui_tx, contacts, msg).await?;
        return Ok(());
    }

    if msg.r#type == "file_ack" {
        handle_file_ack(profile, tui_tx, contacts, msg).await?;
        return Ok(());
    }

    // Normal message.
    let decrypt_result = decrypt_and_verify(msg, profile, contacts);
    let decrypt_error = decrypt_result.as_ref().err().map(|e| e.to_string());
    let (plaintext, verified) = decrypt_result.unwrap_or_else(|e| {
        tracing::error!(error=%e, "decrypt/verify failed");
        (String::new(), false)
    });

    let body_for_display = body_for_inbound_display(&plaintext, decrypt_error.as_deref());

    let contact_name = contact_name_for_pubkey(contacts, &msg.from, verified);

    record_inbound_chat_plaintext(
        profile,
        tui_tx,
        &contact_name,
        &body_for_display,
        msg.timestamp_ms,
        verified,
    )
    .await?;

    if verified {
        tracing::info!(recv=true, v=%msg.v, "message received and verified");
    } else {
        tracing::warn!(from=%msg.from, "signature verification FAILED");
    }

    Ok(())
}

pub(crate) async fn record_inbound_chat_plaintext(
    profile: &Path,
    tui_tx: &mpsc::Sender<TuiEvent>,
    contact_name: &str,
    plaintext: &str,
    timestamp_ms: u128,
    verified: bool,
) -> Result<()> {
    let status = if verified {
        DeliveryStatus::Delivered
    } else {
        DeliveryStatus::Failed
    };

    // Always try to parse as a group message first, regardless of verified
    // status. The verified flag affects delivery status only, not routing.
    // Never fall through to contact storage after a valid group payload: that
    // produces GUI PMs containing raw {"kind":"group_message", ...} JSON.
    if let Some(payload) = parse_group_message_payload(plaintext) {
        let group = discover_or_update_group(
            profile,
            &payload.group_id,
            &payload.group_title,
            contact_name,
            &payload.members,
        )?;
        store_message_for_conversation(
            profile,
            "in",
            contact_name,
            "",
            &payload.body,
            timestamp_ms,
            status,
            "group",
            &group.id,
        )?;
        let _ = tui_tx
            .send(TuiEvent::InboundGroupMessage {
                group_id: group.id,
                group_title: group.title,
                contact: contact_name.to_string(),
                body: payload.body,
                timestamp_ms,
                verified,
            })
            .await;
        return Ok(());
    }

    store_message(
        profile,
        "in",
        contact_name,
        "",
        plaintext,
        timestamp_ms,
        status,
    )?;

    let _ = tui_tx
        .send(TuiEvent::InboundMessage {
            contact: contact_name.to_string(),
            body: plaintext.to_string(),
            timestamp_ms,
            verified,
        })
        .await;
    Ok(())
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

fn parse_group_message_payload(plaintext: &str) -> Option<GroupMessagePayload> {
    fn valid(payload: GroupMessagePayload) -> Option<GroupMessagePayload> {
        (payload.kind == "group_message").then_some(payload)
    }

    if let Ok(payload) = serde_json::from_str::<GroupMessagePayload>(plaintext) {
        return valid(payload);
    }

    // Some bad historical rows/paths can contain a JSON string whose content is
    // the actual group payload JSON. Peel that once so routing is still robust.
    if let Ok(inner) = serde_json::from_str::<String>(plaintext) {
        if let Ok(payload) = serde_json::from_str::<GroupMessagePayload>(&inner) {
            return valid(payload);
        }
    }

    None
}

async fn handle_file_offer(
    profile: &Path,
    tui_tx: &mpsc::Sender<TuiEvent>,
    contacts: &ContactsMap,
    msg: &mut ChatMessage,
) -> Result<()> {
    let (plaintext, verified) = decrypt_and_verify(msg, profile, contacts).unwrap_or_else(|e| {
        tracing::error!(error=%e, "decrypt/verify failed");
        (String::new(), false)
    });

    let contact_name = contact_name_for_pubkey(contacts, &msg.from, verified);

    let body_for_display = if verified {
        match serde_json::from_str::<FileOfferPayload>(&plaintext) {
            Ok(offer) => {
                let key = format!("{}:{}", msg.from, offer.hash);
                let mut map = crate::incoming_files_map()
                    .lock()
                    .map_err(|_| anyhow::anyhow!("incoming file map lock poisoned"))?;
                map.insert(
                    key,
                    IncomingFileState {
                        total_chunks: offer.total_chunks,
                        chunks: vec![None; offer.total_chunks],
                    },
                );
                if let Err(e) = crate::persist_incoming_states(profile) {
                    tracing::warn!(error=%e, "failed to persist incoming file offer state");
                }
                format!(
                    "[file offer] {} ({} bytes, {} chunks)",
                    offer.name, offer.size, offer.total_chunks
                )
            }
            Err(_) => format!("[file offer] {plaintext}"),
        }
    } else {
        "[file offer — UNVERIFIED]".to_string()
    };

    store_message(
        profile,
        "in",
        &contact_name,
        "",
        &body_for_display,
        msg.timestamp_ms,
        if verified {
            DeliveryStatus::Delivered
        } else {
            DeliveryStatus::Failed
        },
    )?;

    let _ = tui_tx
        .send(TuiEvent::InboundMessage {
            contact: contact_name,
            body: body_for_display,
            timestamp_ms: msg.timestamp_ms,
            verified,
        })
        .await;

    tracing::info!(recv=true, %msg.r#type, "file offer received");
    Ok(())
}

async fn handle_file_chunk(
    profile: &Path,
    tui_tx: &mpsc::Sender<TuiEvent>,
    contacts: &ContactsMap,
    msg: &mut ChatMessage,
    tor_client: Arc<arti_client::TorClient<tor_rtcompat::PreferredRuntime>>,
) -> Result<()> {
    let (plaintext, _verified) = decrypt_and_verify(msg, profile, contacts).unwrap_or_else(|e| {
        tracing::error!(error=%e, "decrypt/verify failed");
        (String::new(), false)
    });

    // Accept chunks when decryption succeeds even if signature verification fails.
    // This prevents transfer deadlocks when peers have stale Ed25519 contact keys
    // but still share valid X25519 encryption keys.
    if let Ok(chunk) = serde_json::from_str::<FileChunkPayload>(&plaintext) {
        let key = format!("{}:{}", msg.from, chunk.hash);
        let mut completed_data: Option<Vec<u8>> = None;

        {
            let mut map = crate::incoming_files_map()
                .lock()
                .map_err(|_| anyhow::anyhow!("incoming file map lock poisoned"))?;
            let state = map.entry(key.clone()).or_insert_with(|| IncomingFileState {
                total_chunks: chunk.total_chunks,
                chunks: vec![None; chunk.total_chunks],
            });

            if chunk.chunk_index < state.total_chunks {
                if let Ok(bytes) = B64.decode(chunk.data_b64.as_bytes()) {
                    state.chunks[chunk.chunk_index] = Some(bytes);
                }
            }

            if state.chunks.iter().all(|c| c.is_some()) {
                let mut assembled = Vec::new();
                for c in &state.chunks {
                    assembled.extend_from_slice(c.as_ref().unwrap());
                }
                completed_data = Some(assembled);
                map.remove(&key);
            }
        }

        if let Err(e) = crate::persist_incoming_states(profile) {
            tracing::warn!(error=%e, "failed to persist incoming chunk state");
        }

        // Send ACK back.
        let ack = FileAckPayload {
            hash: chunk.hash.clone(),
            chunk_index: chunk.chunk_index,
            total_chunks: chunk.total_chunks,
            status: "received".to_string(),
        };
        let ack_contact = resolve_contact_name_by_pubkey(contacts, &msg.from)
            .ok()
            .or_else(|| {
                if contacts.len() == 1 {
                    let only = contacts.keys().next().cloned();
                    if let Some(ref name) = only {
                        tracing::warn!(
                            contact=%name,
                            from=%msg.from,
                            "falling back to only configured contact for file ACK"
                        );
                    }
                    only
                } else {
                    None
                }
            });

        if let Some(contact_name) = ack_contact {
            let onion = contacts
                .get(&contact_name)
                .map(|c| c.onion.clone())
                .unwrap_or_default();
            if !onion.is_empty() {
                let ack_json = serde_json::to_string(&ack).unwrap_or_default();
                match send_typed_message(
                    profile,
                    &onion,
                    &contact_name,
                    "file_ack",
                    &ack_json,
                    Arc::clone(&tor_client),
                )
                .await
                {
                    Ok(()) => {
                        tracing::info!(
                            contact=%contact_name,
                            chunk_index=chunk.chunk_index,
                            total_chunks=chunk.total_chunks,
                            "file ack sent"
                        );
                    }
                    Err(e) => {
                        tracing::error!(
                            contact=%contact_name,
                            chunk_index=chunk.chunk_index,
                            total_chunks=chunk.total_chunks,
                            error=%e,
                            "file ack send failed"
                        );
                    }
                }
            } else {
                tracing::error!(contact=%contact_name, "file ack skipped: contact onion missing");
            }
        } else {
            tracing::error!(from=%msg.from, contacts=%contacts.len(), "file ack skipped: no contact match for sender pubkey");
        }

        // If completed, verify hash and write file.
        if let Some(data) = completed_data {
            use sha2::Digest;
            let mut h = sha2::Sha256::new();
            h.update(&data);
            let actual_hash = format!("{:x}", h.finalize());

            let contact_name = contact_name_for_pubkey(contacts, &msg.from, true);
            let downloads_dir = profile.join("downloads");
            if let Err(e) = std::fs::create_dir_all(&downloads_dir) {
                tracing::error!(error=%e, "failed to create downloads dir");
            }

            let mut body = format!("[file received failed hash: {}]", chunk.name);
            if actual_hash == chunk.hash {
                let safe_name = std::path::Path::new(&chunk.name)
                    .file_name()
                    .and_then(|n| n.to_str())
                    .filter(|n| !n.is_empty())
                    .unwrap_or("download.bin")
                    .to_string();
                let out_path = downloads_dir.join(&safe_name);
                match write_file_atomically(&out_path, &data) {
                    Ok(_) => {
                        body = format!("[file received: {}]", out_path.display());
                    }
                    Err(e) => {
                        body = format!("[file write failed: {e}]");
                    }
                }
            }

            store_message(
                profile,
                "in",
                &contact_name,
                "",
                &body,
                msg.timestamp_ms,
                DeliveryStatus::Delivered,
            )?;
            let _ = tui_tx
                .send(TuiEvent::InboundMessage {
                    contact: contact_name,
                    body,
                    timestamp_ms: msg.timestamp_ms,
                    verified: true,
                })
                .await;
        }
    }

    Ok(())
}

async fn handle_file_inline(
    profile: &Path,
    tui_tx: &mpsc::Sender<TuiEvent>,
    contacts: &ContactsMap,
    msg: &mut ChatMessage,
) -> Result<()> {
    let (plaintext, verified) = decrypt_and_verify(msg, profile, contacts).unwrap_or_else(|e| {
        tracing::error!(error=%e, "decrypt/verify failed");
        (String::new(), false)
    });

    if let Ok(inline) = serde_json::from_str::<FileInlinePayload>(&plaintext) {
        let contact_name = contact_name_for_pubkey(contacts, &msg.from, verified);
        let downloads_dir = profile.join("downloads");
        if let Err(e) = std::fs::create_dir_all(&downloads_dir) {
            tracing::error!(error=%e, "failed to create downloads dir");
        }

        let mut body = format!("[file receive failed: {}]", inline.name);
        if let Ok(data) = B64.decode(inline.data_b64.as_bytes()) {
            use sha2::Digest;
            let mut h = sha2::Sha256::new();
            h.update(&data);
            let actual_hash = format!("{:x}", h.finalize());
            if actual_hash == inline.hash {
                let safe_name = std::path::Path::new(&inline.name)
                    .file_name()
                    .and_then(|n| n.to_str())
                    .filter(|n| !n.is_empty())
                    .unwrap_or("download.bin")
                    .to_string();
                let out_path = downloads_dir.join(&safe_name);
                match write_file_atomically(&out_path, &data) {
                    Ok(_) => body = format!("[file received: {}]", out_path.display()),
                    Err(e) => body = format!("[file write failed: {e}]"),
                }
            } else {
                body = format!("[file hash mismatch: {}]", inline.name);
            }
        }

        store_message(
            profile,
            "in",
            &contact_name,
            "",
            &body,
            msg.timestamp_ms,
            DeliveryStatus::Delivered,
        )?;
        let _ = tui_tx
            .send(TuiEvent::InboundMessage {
                contact: contact_name,
                body,
                timestamp_ms: msg.timestamp_ms,
                verified,
            })
            .await;
    }

    Ok(())
}

fn ack_is_acceptable(ack: &FileAckPayload) -> bool {
    ack.status == "received" && ack.chunk_index < ack.total_chunks
}

async fn handle_file_ack(
    profile: &Path,
    tui_tx: &mpsc::Sender<TuiEvent>,
    contacts: &ContactsMap,
    msg: &mut ChatMessage,
) -> Result<()> {
    let (plaintext, verified) = decrypt_and_verify(msg, profile, contacts).unwrap_or_else(|e| {
        tracing::error!(error=%e, "decrypt/verify failed");
        (String::new(), false)
    });

    // Accept ACKs when decryption succeeds even if signature verification fails.
    // Signature/key drift should not deadlock transfer progress.
    let mut accepted = false;
    if let Ok(ack) = serde_json::from_str::<FileAckPayload>(&plaintext) {
        if ack_is_acceptable(&ack) {
            if let Ok(mut set) = crate::file_ack_set().lock() {
                set.insert(crate::ack_key(&ack.hash, ack.chunk_index));
                accepted = true;
            }
        }
    }
    let contact_name = contact_name_for_pubkey(contacts, &msg.from, verified);
    let body = if accepted {
        format!("[file ack] {plaintext}")
    } else {
        format!("[file ack ignored] {plaintext}")
    };
    let _ = tui_tx
        .send(TuiEvent::InboundMessage {
            contact: contact_name,
            body,
            timestamp_ms: msg.timestamp_ms,
            verified,
        })
        .await;

    Ok(())
}

fn write_file_atomically(path: &std::path::Path, data: &[u8]) -> Result<()> {
    use std::io::Write;

    let parent = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("output path has no parent: {}", path.display()))?;
    std::fs::create_dir_all(parent)?;

    let pid = std::process::id();
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let base_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .filter(|n| !n.is_empty())
        .unwrap_or("download.bin");
    let tmp_name = format!(".{}.{}.{}.part", base_name, pid, ts);
    let tmp_path = parent.join(tmp_name);

    {
        let mut f = std::fs::File::create(&tmp_path)?;
        f.write_all(data)?;
        f.sync_all()?;
    }

    std::fs::rename(&tmp_path, path)?;
    Ok(())
}

/// Resolve a human-readable contact name from a publisher's public key.
fn contact_name_for_pubkey(contacts: &ContactsMap, pubkey: &str, verified: bool) -> String {
    contacts
        .values()
        .find(|c| c.pubkey_b64 == pubkey)
        .map(|c| c.name.clone())
        .unwrap_or_else(|| {
            if verified {
                "verified-peer".into()
            } else {
                pubkey.to_string()
            }
        })
}

fn body_for_inbound_display(plaintext: &str, decrypt_error: Option<&str>) -> String {
    if !plaintext.is_empty() {
        return plaintext.to_string();
    }

    match decrypt_error {
        Some(e) => format!("[decryption failed: {e}]"),
        None => "[decryption failed]".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::{ack_is_acceptable, body_for_inbound_display, write_file_atomically};
    use crate::FileAckPayload;

    #[test]
    fn atomic_write_replaces_file_contents() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("recv.bin");

        write_file_atomically(&out, b"hello").unwrap();
        let first = std::fs::read(&out).unwrap();
        assert_eq!(first, b"hello");

        write_file_atomically(&out, b"world").unwrap();
        let second = std::fs::read(&out).unwrap();
        assert_eq!(second, b"world");
    }

    #[test]
    fn ack_acceptance_requires_received_and_valid_index() {
        let ok = FileAckPayload {
            hash: "abc".to_string(),
            chunk_index: 1,
            total_chunks: 4,
            status: "received".to_string(),
        };
        assert!(ack_is_acceptable(&ok));

        let bad_status = FileAckPayload {
            status: "error".to_string(),
            ..ok
        };
        assert!(!ack_is_acceptable(&bad_status));

        let out_of_range = FileAckPayload {
            hash: "abc".to_string(),
            chunk_index: 4,
            total_chunks: 4,
            status: "received".to_string(),
        };
        assert!(!ack_is_acceptable(&out_of_range));
    }

    #[test]
    fn failed_inbound_decrypt_has_visible_history_body() {
        assert_eq!(
            body_for_inbound_display("", Some("unknown sender pubkey: abc")),
            "[decryption failed: unknown sender pubkey: abc]"
        );
        assert_eq!(body_for_inbound_display("hello", None), "hello");
    }
}

#[cfg(test)]
mod group_chat_tests {
    use super::*;
    use crate::{contact_add, init_profile, load_group_history, load_groups, load_history};

    #[tokio::test]
    async fn inbound_group_payload_stores_group_history_not_direct_history() {
        let dir = tempfile::tempdir().unwrap();
        init_profile(dir.path()).unwrap();
        let pk = B64.encode([1u8; 32]);
        let xpk = B64.encode([2u8; 32]);
        contact_add(
            dir.path(),
            "alice",
            "stqclefnkl4wfmdsz627hlfwu2xwgrk3sb6sgegfq44auik3pz7jmyqd.onion",
            &pk,
            &xpk,
        )
        .unwrap();
        contact_add(
            dir.path(),
            "bob",
            "psrntpu56hilbftscupr6f4ujxb6kjn6n2qy4366sen4lqkqpspjezid.onion",
            &pk,
            &xpk,
        )
        .unwrap();
        let (tx, mut rx) = mpsc::channel::<TuiEvent>(4);
        let payload = serde_json::json!({
            "kind": "group_message",
            "group_id": "g-ops",
            "group_title": "Ops",
            "members": ["bob", "local-self-is-not-a-contact"],
            "body": "ops hello"
        })
        .to_string();

        record_inbound_chat_plaintext(dir.path(), &tx, "alice", &payload, 123, true)
            .await
            .unwrap();

        let groups = load_groups(dir.path()).unwrap();
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].id, "g-ops");
        assert_eq!(groups[0].title, "Ops");
        assert!(groups[0].members.iter().any(|m| m.contact == "alice"));
        assert!(groups[0].members.iter().any(|m| m.contact == "bob"));
        assert!(!groups[0]
            .members
            .iter()
            .any(|m| m.contact == "local-self-is-not-a-contact"));

        let group_rows = load_group_history(dir.path(), "g-ops", 10).unwrap();
        assert_eq!(group_rows.len(), 1);
        assert_eq!(group_rows[0].body, "ops hello");
        assert_eq!(group_rows[0].contact, "alice");
        assert_eq!(group_rows[0].conversation_kind, "group");
        assert_eq!(group_rows[0].conversation_id, "g-ops");
        assert!(load_history(dir.path(), Some("alice"), 10)
            .unwrap()
            .is_empty());

        match rx.recv().await.unwrap() {
            TuiEvent::InboundGroupMessage {
                group_id,
                group_title,
                contact,
                body,
                ..
            } => {
                assert_eq!(group_id, "g-ops");
                assert_eq!(group_title, "Ops");
                assert_eq!(contact, "alice");
                assert_eq!(body, "ops hello");
            }
            other => panic!("expected group event, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn inbound_group_payload_unverified_still_stores_as_group() {
        // Regression: when verified=false (e.g. v3 from unknown sender),
        // GroupMessagePayload must still be routed to group history, not
        // stored as a contact PM with raw JSON.
        let dir = tempfile::tempdir().unwrap();
        init_profile(dir.path()).unwrap();
        let pk = B64.encode([1u8; 32]);
        let xpk = B64.encode([2u8; 32]);
        contact_add(
            dir.path(),
            "bob",
            "stqclefnkl4wfmdsz627hlfwu2xwgrk3sb6sgegfq44auik3pz7jmyqd.onion",
            &pk,
            &xpk,
        )
        .unwrap();
        let (tx, mut rx) = mpsc::channel::<TuiEvent>(4);
        let payload = serde_json::json!({
            "kind": "group_message",
            "group_id": "g-offtopic",
            "group_title": "Offtopic",
            "members": ["bob"],
            "body": "hello from unknown"
        })
        .to_string();

        record_inbound_chat_plaintext(dir.path(), &tx, "stranger", &payload, 456, false)
            .await
            .unwrap();

        // Must appear in group history, not in stranger's DM history.
        let groups = load_groups(dir.path()).unwrap();
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].id, "g-offtopic");

        let group_rows = load_group_history(dir.path(), "g-offtopic", 10).unwrap();
        assert_eq!(group_rows.len(), 1);
        assert_eq!(group_rows[0].body, "hello from unknown");
        assert_eq!(group_rows[0].contact, "stranger");
        assert_eq!(group_rows[0].conversation_kind, "group");

        // Must NOT appear as a contact PM.
        assert!(load_history(dir.path(), Some("stranger"), 10)
            .unwrap()
            .is_empty());

        // Must emit InboundGroupMessage event.
        match rx.recv().await.unwrap() {
            TuiEvent::InboundGroupMessage {
                group_id,
                body,
                contact,
                ..
            } => {
                assert_eq!(group_id, "g-offtopic");
                assert_eq!(body, "hello from unknown");
                assert_eq!(contact, "stranger");
            }
            other => panic!("expected group event, got {other:?}"),
        }
    }
}
