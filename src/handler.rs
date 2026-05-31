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
    decrypt_and_verify, resolve_contact_name_by_pubkey, send_typed_message, store_message,
    ChatMessage, ContactsMap, DeliveryStatus, FileAckPayload, FileChunkPayload, FileOfferPayload,
    IncomingFileState, TuiEvent,
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

    if msg.r#type == "file_ack" {
        handle_file_ack(tui_tx, contacts, msg).await?;
        return Ok(());
    }

    // Normal message.
    let decrypt_result = decrypt_and_verify(msg, profile, contacts);
    let decrypt_error = decrypt_result.as_ref().err().map(|e| e.to_string());
    let (plaintext, verified) = decrypt_result.unwrap_or_else(|e| {
        tracing::error!(error=%e, "decrypt/verify failed");
        (String::new(), false)
    });

    let body_for_display = if plaintext.is_empty() {
        match decrypt_error {
            Some(e) => format!("[decryption failed: {e}]"),
            None => "[decryption failed]".to_string(),
        }
    } else {
        plaintext.clone()
    };

    let contact_name = contact_name_for_pubkey(contacts, &msg.from, verified);

    store_message(
        profile,
        "in",
        &contact_name,
        "",
        &plaintext,
        msg.timestamp_ms,
        if verified {
            DeliveryStatus::Delivered
        } else {
            DeliveryStatus::Failed
        },
    )?;

    let _ = tui_tx
        .send(TuiEvent::InboundMessage {
            contact: contact_name.clone(),
            body: body_for_display,
            timestamp_ms: msg.timestamp_ms,
            verified,
        })
        .await;

    if verified {
        tracing::info!(recv=true, v=%msg.v, "message received and verified");
    } else {
        tracing::warn!(from=%msg.from, "signature verification FAILED");
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

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
    let (plaintext, verified) = decrypt_and_verify(msg, profile, contacts).unwrap_or_else(|e| {
        tracing::error!(error=%e, "decrypt/verify failed");
        (String::new(), false)
    });

    if !verified {
        return Ok(());
    }

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
        if let Ok(contact_name) = resolve_contact_name_by_pubkey(contacts, &msg.from) {
            let onion = contacts
                .get(&contact_name)
                .map(|c| c.onion.clone())
                .unwrap_or_default();
            if !onion.is_empty() {
                let _ = send_typed_message(
                    profile,
                    &onion,
                    &contact_name,
                    "file_ack",
                    &serde_json::to_string(&ack).unwrap_or_default(),
                    Arc::clone(&tor_client),
                )
                .await;
            }
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
                match std::fs::write(&out_path, &data) {
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

async fn handle_file_ack(
    tui_tx: &mpsc::Sender<TuiEvent>,
    contacts: &ContactsMap,
    msg: &mut ChatMessage,
) -> Result<()> {
    // file_ack doesn't need profile for decryption key lookup; we just need
    // the sender's pubkey. Pass an empty-profile hint — decrypt_and_verify
    // uses msg.from to find the contact.
    let (plaintext, verified) =
        decrypt_and_verify(msg, Path::new("."), contacts).unwrap_or_else(|e| {
            tracing::error!(error=%e, "decrypt/verify failed");
            (String::new(), false)
        });

    if verified {
        if let Ok(ack) = serde_json::from_str::<FileAckPayload>(&plaintext) {
            if let Ok(mut set) = crate::file_ack_set().lock() {
                set.insert(crate::ack_key(&ack.hash, ack.chunk_index));
            }
        }
        let contact_name = contact_name_for_pubkey(contacts, &msg.from, true);
        let body = format!("[file ack] {plaintext}");
        let _ = tui_tx
            .send(TuiEvent::InboundMessage {
                contact: contact_name,
                body,
                timestamp_ms: msg.timestamp_ms,
                verified: true,
            })
            .await;
    }

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
