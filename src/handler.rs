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

use crate::transport::tor::SharedTransferState;
use crate::transport::tor::TorTransport;
use crate::{
    contact_is_blocked, decrypt_and_verify, discover_or_update_group,
    mark_delivered_by_fingerprint, mark_read_up_to, message_replay_fingerprint,
    note_contact_presence, resolve_contact_name_by_pubkey, send_delivered_receipt, send_sync_ack,
    send_sync_inventory, send_sync_items, send_sync_request, send_typed_message,
    set_contact_transport_prop, store_message, store_message_for_conversation,
    store_message_for_conversation_expiring, ChatMessage, ContactsMap, DeliveryStatus,
    FileAckPayload, FileChunkPayload, FileInlinePayload, FileOfferPayload, GroupMessagePayload,
    IncomingFileState, PresencePayload, ReceiptPayload, TransportPropsPayload, TuiEvent,
};

type TorClientArc = Arc<arti_client::TorClient<tor_rtcompat::PreferredRuntime>>;

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
    transfer_state: &SharedTransferState,
) -> Result<()> {
    if contact_is_blocked(contacts, msg) {
        tracing::info!(from=%msg.from, "dropping message from blocked contact");
        return Ok(());
    }

    if msg.r#type == "file_offer" {
        handle_file_offer(profile, tui_tx, contacts, msg, transfer_state).await?;
        return Ok(());
    }

    if msg.r#type == "file_chunk" {
        handle_file_chunk(profile, tui_tx, contacts, msg, tor_client, transfer_state).await?;
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

    if msg.r#type == "group_leave" {
        return handle_group_leave(profile, tui_tx, contacts, msg).await;
    }

    if msg.r#type == "group_deleted" {
        return handle_group_deleted(profile, tui_tx, contacts, msg).await;
    }

    if msg.r#type == "receipt" {
        return handle_receipt(profile, contacts, msg).await;
    }

    if msg.r#type == "presence" {
        return handle_presence(profile, contacts, msg);
    }

    if msg.r#type == "device_list" {
        return handle_device_list_push(profile, contacts, msg);
    }

    if msg.r#type == "transport_props" {
        handle_transport_props(profile, contacts, msg).await?;
        if let Ok(contact_name) = resolve_contact_name_by_pubkey(contacts, &msg.from) {
            let profile = profile.to_path_buf();
            tokio::spawn(async move {
                if let Err(e) = send_sync_inventory(&profile, &contact_name, true, tor_client).await
                {
                    tracing::debug!(error=%e, "BSP inventory initiation failed");
                }
            });
        }
        return Ok(());
    }

    if msg.r#type == "sync_inventory" {
        return handle_sync_inventory(profile, contacts, msg, tor_client).await;
    }
    if msg.r#type == "sync_request" {
        return handle_sync_request(profile, contacts, msg, tor_client).await;
    }
    if msg.r#type == "sync_item" {
        return handle_sync_item(profile, tui_tx, contacts, msg, tor_client).await;
    }
    if msg.r#type == "sync_ack" {
        return handle_sync_ack(profile, contacts, msg).await;
    }

    // Normal message. Pass the Tor client so a delivery receipt can be sent back,
    // then use this authenticated contact activity as a bidirectional sync opportunity.
    let contact_name = resolve_contact_name_by_pubkey(contacts, &msg.from);
    handle_text_message(
        profile,
        tui_tx,
        contacts,
        msg,
        Some(Arc::clone(&tor_client)),
    )
    .await?;
    if let Ok(contact_name) = contact_name {
        let profile = profile.to_path_buf();
        tokio::spawn(async move {
            if let Err(e) = send_sync_inventory(&profile, &contact_name, true, tor_client).await {
                tracing::debug!(error=%e, "BSP inventory initiation failed");
            }
        });
    }
    Ok(())
}

/// Persist transport addresses shared by an authenticated, accepted contact.
/// The typed payload is never stored as chat history or shown in a client.
pub(crate) async fn handle_transport_props(
    profile: &Path,
    contacts: &ContactsMap,
    msg: &mut ChatMessage,
) -> Result<()> {
    let (plaintext, verified) = match decrypt_and_verify(msg, profile, contacts) {
        Ok(value) => value,
        Err(e) => {
            tracing::debug!(error=%e, "transport properties decrypt/verify failed");
            return Ok(());
        }
    };
    if !verified {
        tracing::warn!(from=%msg.from, "dropping unverified transport properties");
        return Ok(());
    }

    let Some(contact) = contacts
        .values()
        .find(|contact| contact.pubkey_b64 == msg.from && !contact.pending && !contact.blocked)
    else {
        tracing::debug!(from=%msg.from, "dropping transport properties from non-contact");
        return Ok(());
    };

    let payload: TransportPropsPayload = match serde_json::from_str(&plaintext) {
        Ok(payload) => payload,
        Err(e) => {
            tracing::debug!(error=%e, "malformed transport properties payload");
            return Ok(());
        }
    };
    if payload.kind != "transport_props" || payload.properties.len() > 16 {
        tracing::debug!("invalid transport properties payload");
        return Ok(());
    }

    for property in payload.properties {
        if property.transport.is_empty()
            || property.transport.len() > 64
            || property.value.is_empty()
            || property.value.len() > 1024
        {
            continue;
        }
        if property.transport == "lan" {
            let Ok(addr) = property.value.parse::<std::net::SocketAddr>() else {
                continue;
            };
            if addr.ip().is_unspecified() || addr.ip().is_multicast() || addr.port() == 0 {
                continue;
            }
            if set_contact_transport_prop(
                profile,
                &contact.name,
                &property.transport,
                &property.value,
                msg.timestamp_ms,
            )? {
                crate::transport::lan::PEERS.note(&contact.pubkey_b64, addr, msg.timestamp_ms);
            }
        } else {
            set_contact_transport_prop(
                profile,
                &contact.name,
                &property.transport,
                &property.value,
                msg.timestamp_ms,
            )?;
        }
    }
    Ok(())
}

fn decrypt_authenticated_sync(
    profile: &Path,
    contacts: &ContactsMap,
    msg: &mut ChatMessage,
) -> Result<Option<(String, String)>> {
    let (plaintext, verified) = match decrypt_and_verify(msg, profile, contacts) {
        Ok(value) => value,
        Err(e) => {
            tracing::debug!(error=%e, "sync decrypt/verify failed");
            return Ok(None);
        }
    };
    if !verified {
        return Ok(None);
    }
    let Some(contact) = contacts
        .values()
        .find(|contact| contact.pubkey_b64 == msg.from && !contact.pending && !contact.blocked)
    else {
        return Ok(None);
    };
    Ok(Some((plaintext, contact.name.clone())))
}

pub(crate) async fn handle_sync_inventory(
    profile: &Path,
    contacts: &ContactsMap,
    msg: &mut ChatMessage,
    tor_client: TorClientArc,
) -> Result<()> {
    let Some((plaintext, contact_name)) = decrypt_authenticated_sync(profile, contacts, msg)?
    else {
        return Ok(());
    };
    let inventory: crate::sync::SyncInventoryPayload = serde_json::from_str(&plaintext)?;
    let request = crate::sync::missing_from_inventory(profile, &msg.from, &inventory)?;
    if !request.ids.is_empty() {
        send_sync_request(profile, &contact_name, &request, Arc::clone(&tor_client)).await?;
    }
    if inventory.reply {
        send_sync_inventory(profile, &contact_name, false, tor_client).await?;
    }
    Ok(())
}

pub(crate) async fn handle_sync_request(
    profile: &Path,
    contacts: &ContactsMap,
    msg: &mut ChatMessage,
    tor_client: TorClientArc,
) -> Result<()> {
    let Some((plaintext, contact_name)) = decrypt_authenticated_sync(profile, contacts, msg)?
    else {
        return Ok(());
    };
    let request: crate::sync::SyncRequestPayload = serde_json::from_str(&plaintext)?;
    send_sync_items(profile, &contact_name, &request, tor_client).await
}

pub(crate) async fn handle_sync_item(
    profile: &Path,
    tui_tx: &mpsc::Sender<TuiEvent>,
    contacts: &ContactsMap,
    msg: &mut ChatMessage,
    tor_client: TorClientArc,
) -> Result<()> {
    let Some((plaintext, contact_name)) = decrypt_authenticated_sync(profile, contacts, msg)?
    else {
        return Ok(());
    };
    let mut item: crate::sync::SyncItemPayload = serde_json::from_str(&plaintext)?;
    if item.kind != "sync_item"
        || item.message.from != msg.from
        || item.message.r#type != "sync_chat"
    {
        return Ok(());
    }
    if !crate::sync::mark_received(profile, &msg.from, &item.id)? {
        return send_sync_ack(profile, &contact_name, &item.id, tor_client).await;
    }
    if let Err(e) = handle_text_message(
        profile,
        tui_tx,
        contacts,
        &mut item.message,
        Some(Arc::clone(&tor_client)),
    )
    .await
    {
        crate::sync::unmark_received(profile, &msg.from, &item.id)?;
        return Err(e);
    }
    send_sync_ack(profile, &contact_name, &item.id, tor_client).await
}

pub(crate) async fn handle_sync_ack(
    profile: &Path,
    contacts: &ContactsMap,
    msg: &mut ChatMessage,
) -> Result<()> {
    let Some((plaintext, contact_name)) = decrypt_authenticated_sync(profile, contacts, msg)?
    else {
        return Ok(());
    };
    let ack: crate::sync::SyncAckPayload = serde_json::from_str(&plaintext)?;
    if ack.kind == "sync_ack" {
        crate::sync::ack_outbound(profile, &contact_name, &ack.id)?;
    }
    Ok(())
}

/// Handle an inbound delivery/read receipt: verify it, then update the status of
/// the outbound message(s) it acknowledges. Receipts are never stored or shown.
pub(crate) async fn handle_receipt(
    profile: &Path,
    contacts: &ContactsMap,
    msg: &mut ChatMessage,
) -> Result<()> {
    let (plaintext, verified) = match decrypt_and_verify(msg, profile, contacts) {
        Ok(v) => v,
        Err(e) => {
            tracing::debug!(error=%e, "receipt decrypt/verify failed");
            return Ok(());
        }
    };
    if !verified {
        tracing::warn!(from=%msg.from, "dropping unverified receipt");
        return Ok(());
    }
    let payload: ReceiptPayload = match serde_json::from_str(&plaintext) {
        Ok(p) => p,
        Err(e) => {
            tracing::debug!(error=%e, "malformed receipt payload");
            return Ok(());
        }
    };
    let contact_name = contact_name_for_pubkey(profile, contacts, &msg.from, verified);
    match payload.state.as_str() {
        "delivered" => {
            if let Some(fp) = payload.msg_fingerprint {
                let changed = mark_delivered_by_fingerprint(profile, &fp).unwrap_or(false);
                tracing::info!(contact = %contact_name, changed, "receipt: delivered");
            }
        }
        "read" => {
            if let Some(up_to) = payload.up_to_ms {
                let n = mark_read_up_to(profile, &contact_name, up_to).unwrap_or(0);
                tracing::info!(contact = %contact_name, marked = n, "receipt: read");
            }
        }
        other => tracing::debug!(state = %other, "unknown receipt state"),
    }
    Ok(())
}

/// Handle an inbound presence heartbeat (A7): verify it came from an accepted
/// contact, then record their state with a receiver-stamped validity window.
/// Never stored as a message or shown; `seq` gates out stale/reordered packets.
/// A contact pushed us their updated signed device list (they linked or revoked
/// a device). Verify it came from that contact's account and store it, so our
/// send fan-out + inbound attribution learn their new device set.
pub(crate) fn handle_device_list_push(
    profile: &Path,
    contacts: &ContactsMap,
    msg: &mut ChatMessage,
) -> Result<()> {
    let (plaintext, verified) = match decrypt_and_verify(msg, profile, contacts) {
        Ok(v) => v,
        Err(_) => return Ok(()),
    };
    if !verified {
        return Ok(());
    }
    // Map the sending device to an accepted, non-blocked contact, then apply the
    // list only if it is signed by that contact's account (store enforces this).
    let (name, account) = match crate::resolve_contact_by_sender(profile, contacts, &msg.from) {
        Some(c) if !c.pending && !c.blocked => (c.name.clone(), c.pubkey_b64.clone()),
        _ => return Ok(()),
    };
    crate::store_pushed_device_list(profile, &name, &account, &plaintext)
}

pub(crate) fn handle_presence(
    profile: &Path,
    contacts: &ContactsMap,
    msg: &mut ChatMessage,
) -> Result<()> {
    let (plaintext, verified) = match decrypt_and_verify(msg, profile, contacts) {
        Ok(v) => v,
        Err(_) => return Ok(()),
    };
    if !verified {
        return Ok(());
    }
    // Presence is only meaningful from an accepted, non-blocked contact.
    if !contacts
        .values()
        .any(|c| c.pubkey_b64 == msg.from && !c.pending && !c.blocked)
    {
        return Ok(());
    }
    let payload: PresencePayload = match serde_json::from_str(&plaintext) {
        Ok(p) => p,
        Err(_) => return Ok(()),
    };
    if payload.kind != "presence" {
        return Ok(());
    }
    let state = match payload.state.as_str() {
        "online" | "away" => payload.state.as_str(),
        _ => return Ok(()),
    };
    // Clamp the sender-chosen TTL so a malicious contact cannot claim to be
    // "online" for an unbounded time. Stamp validity from OUR clock.
    let ttl = payload.ttl_ms.min(crate::PRESENCE_TTL_MS.saturating_mul(4)) as u128;
    let now = crate::now_ms_i64().unwrap_or(0).max(0) as u128;
    note_contact_presence(
        profile,
        &msg.from,
        state,
        now + ttl,
        payload.seq,
        &payload.status,
    )?;
    Ok(())
}

/// Decrypt/verify a normal text message, resolve the sender's contact name, and
/// persist + notify. Split out of [`handle_inbound`] so the text path is testable
/// without a `TorClient` (only file chunks need one, for ACKs).
pub(crate) async fn handle_text_message(
    profile: &Path,
    tui_tx: &mpsc::Sender<TuiEvent>,
    contacts: &ContactsMap,
    msg: &mut ChatMessage,
    tor_client: Option<TorClientArc>,
) -> Result<()> {
    let decrypt_result = decrypt_and_verify(msg, profile, contacts);
    // A replay (e.g. a duplicate carrier delivery after a lost ack) is expected,
    // not an error to surface — drop it silently instead of storing a
    // "[decryption failed]" row in the conversation.
    if decrypt_result
        .as_ref()
        .err()
        .is_some_and(|e| e.downcast_ref::<crate::ReplayedMessage>().is_some())
    {
        return Ok(());
    }
    let decrypt_error = decrypt_result.as_ref().err().map(|e| e.to_string());
    let (plaintext, verified) = decrypt_result.unwrap_or_else(|e| {
        tracing::error!(error=%e, "decrypt/verify failed");
        (String::new(), false)
    });

    let body_for_display = body_for_inbound_display(&plaintext, decrypt_error.as_deref());

    let contact_name = contact_name_for_pubkey(profile, contacts, &msg.from, verified);

    record_inbound_chat_plaintext(
        profile,
        tui_tx,
        &contact_name,
        &body_for_display,
        msg.timestamp_ms,
        verified,
        msg.expires_at_ms,
    )
    .await?;

    if verified {
        tracing::info!(recv=true, v=%msg.v, "message received and verified");
        // Hearing from a contact proves they are reachable right now — flush any
        // messages we have queued for them instead of waiting out the backoff.
        match crate::retry_wake_contact(profile, &contact_name) {
            Ok(n) if n > 0 => {
                tracing::info!(contact = %contact_name, woke = n, "retry: peer online, flushing queue")
            }
            _ => {}
        }
        // Acknowledge receipt to a known (non-pending) contact. Fire-and-forget so
        // inbound processing is never blocked on a Tor round-trip.
        if let Some(tc) = tor_client {
            if let Some(contact) = contacts
                .values()
                .find(|c| c.pubkey_b64 == msg.from && !c.pending && !c.onion.is_empty())
            {
                let fp = message_replay_fingerprint(msg);
                let name = contact.name.clone();
                let onion = contact.onion.clone();
                let profile = profile.to_path_buf();
                tokio::spawn(async move {
                    if let Err(e) = send_delivered_receipt(&profile, &name, &onion, &fp, tc).await {
                        tracing::debug!(error=%e, "failed to send delivery receipt");
                    }
                });
            }
        }
    } else {
        tracing::warn!(from=%msg.from, "signature verification FAILED");
    }

    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn record_inbound_chat_plaintext(
    profile: &Path,
    tui_tx: &mpsc::Sender<TuiEvent>,
    contact_name: &str,
    plaintext: &str,
    timestamp_ms: u128,
    verified: bool,
    expires_at_ms: Option<u128>,
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
        store_message_for_conversation_expiring(
            profile,
            "in",
            contact_name,
            "",
            &payload.body,
            timestamp_ms,
            status,
            "group",
            &group.id,
            expires_at_ms,
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

    store_message_for_conversation_expiring(
        profile,
        "in",
        contact_name,
        "",
        plaintext,
        timestamp_ms,
        status,
        "contact",
        contact_name,
        expires_at_ms,
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

/// Store an inbound file-related message under the correct conversation and emit
/// the matching UI event: the group (when the file was sent to a group) or the
/// sender's 1:1 PM otherwise.
#[allow(clippy::too_many_arguments)]
async fn store_file_message(
    profile: &Path,
    tui_tx: &mpsc::Sender<TuiEvent>,
    contact_name: &str,
    group: Option<(&str, &str)>,
    body: &str,
    timestamp_ms: u128,
    status: DeliveryStatus,
    verified: bool,
) -> Result<()> {
    match group {
        Some((group_id, group_title)) if !group_id.is_empty() => {
            let g = discover_or_update_group(profile, group_id, group_title, contact_name, &[])?;
            store_message_for_conversation(
                profile,
                "in",
                contact_name,
                "",
                body,
                timestamp_ms,
                status,
                "group",
                &g.id,
            )?;
            let _ = tui_tx
                .send(TuiEvent::InboundGroupMessage {
                    group_id: g.id,
                    group_title: g.title,
                    contact: contact_name.to_string(),
                    body: body.to_string(),
                    timestamp_ms,
                    verified,
                })
                .await;
        }
        _ => {
            store_message(profile, "in", contact_name, "", body, timestamp_ms, status)?;
            let _ = tui_tx
                .send(TuiEvent::InboundMessage {
                    contact: contact_name.to_string(),
                    body: body.to_string(),
                    timestamp_ms,
                    verified,
                })
                .await;
        }
    }
    Ok(())
}

async fn handle_file_offer(
    profile: &Path,
    tui_tx: &mpsc::Sender<TuiEvent>,
    contacts: &ContactsMap,
    msg: &mut ChatMessage,
    transfer_state: &SharedTransferState,
) -> Result<()> {
    let (plaintext, verified) = decrypt_and_verify(msg, profile, contacts).unwrap_or_else(|e| {
        tracing::error!(error=%e, "decrypt/verify failed");
        (String::new(), false)
    });

    let contact_name = contact_name_for_pubkey(profile, contacts, &msg.from, verified);

    let mut offer_group: Option<(String, String)> = None;
    let body_for_display = if verified {
        match serde_json::from_str::<FileOfferPayload>(&plaintext) {
            Ok(offer) => {
                if let Err(e) = crate::validate_total_chunks(offer.size, offer.total_chunks) {
                    tracing::warn!(from=%msg.from, error=%e, "rejecting file offer");
                    return Ok(());
                }
                if let Some(id) = offer.group_id.clone() {
                    let title = offer.group_title.clone().unwrap_or_else(|| id.clone());
                    offer_group = Some((id, title));
                }
                let key = format!("{}:{}", msg.from, offer.hash);
                let snapshot = {
                    let mut state = transfer_state.lock().await;
                    state.incoming_files.insert(
                        key,
                        IncomingFileState {
                            total_chunks: offer.total_chunks,
                            chunks: vec![None; offer.total_chunks],
                            group_id: offer.group_id.clone(),
                            group_title: offer.group_title.clone(),
                        },
                    );
                    state.incoming_files.clone()
                };
                if let Err(e) = crate::persist_incoming_states_snapshot(profile, &snapshot) {
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

    let group_ref = offer_group.as_ref().map(|(i, t)| (i.as_str(), t.as_str()));
    store_file_message(
        profile,
        tui_tx,
        &contact_name,
        group_ref,
        &body_for_display,
        msg.timestamp_ms,
        if verified {
            DeliveryStatus::Delivered
        } else {
            DeliveryStatus::Failed
        },
        verified,
    )
    .await?;

    tracing::info!(recv=true, %msg.r#type, "file offer received");
    Ok(())
}

async fn handle_file_chunk(
    profile: &Path,
    tui_tx: &mpsc::Sender<TuiEvent>,
    contacts: &ContactsMap,
    msg: &mut ChatMessage,
    tor_client: Arc<arti_client::TorClient<tor_rtcompat::PreferredRuntime>>,
    transfer_state: &SharedTransferState,
) -> Result<()> {
    let (plaintext, _verified) = decrypt_and_verify(msg, profile, contacts).unwrap_or_else(|e| {
        tracing::error!(error=%e, "decrypt/verify failed");
        (String::new(), false)
    });

    // Accept chunks when decryption succeeds even if signature verification fails.
    // This prevents transfer deadlocks when peers have stale Ed25519 contact keys
    // but still share valid X25519 encryption keys.
    if let Ok(chunk) = serde_json::from_str::<FileChunkPayload>(&plaintext) {
        // Bound the claimed chunk count before allocating vec![None; total_chunks].
        // A bare chunk carries no file size, so only the absolute cap applies here;
        // the file_offer path validated total_chunks against the offered size.
        if let Err(e) = crate::validate_total_chunks(0, chunk.total_chunks) {
            tracing::warn!(from=%msg.from, error=%e, "rejecting file chunk");
            return Ok(());
        }
        if chunk.chunk_index >= chunk.total_chunks {
            tracing::warn!(
                from=%msg.from,
                chunk_index = chunk.chunk_index,
                total_chunks = chunk.total_chunks,
                "rejecting file chunk: index out of range"
            );
            return Ok(());
        }
        let key = format!("{}:{}", msg.from, chunk.hash);
        let mut completed_data: Option<Vec<u8>> = None;
        // Group context (from the offer) for a transfer that completes on this chunk.
        let mut completed_group: Option<(String, String)> = None;

        let snapshot = {
            let mut state = transfer_state.lock().await;
            let file_state =
                state
                    .incoming_files
                    .entry(key.clone())
                    .or_insert_with(|| IncomingFileState {
                        total_chunks: chunk.total_chunks,
                        chunks: vec![None; chunk.total_chunks],
                        group_id: None,
                        group_title: None,
                    });

            if chunk.chunk_index < file_state.total_chunks {
                if let Ok(bytes) = B64.decode(chunk.data_b64.as_bytes()) {
                    file_state.chunks[chunk.chunk_index] = Some(bytes);
                }
            }

            if file_state.chunks.iter().all(|c| c.is_some()) {
                let mut assembled = Vec::new();
                for c in &file_state.chunks {
                    assembled.extend_from_slice(c.as_ref().unwrap());
                }
                completed_data = Some(assembled);
                if let Some(id) = file_state.group_id.clone() {
                    let title = file_state.group_title.clone().unwrap_or_else(|| id.clone());
                    completed_group = Some((id, title));
                }
                state.incoming_files.remove(&key);
            }
            // Snapshot the live state (the source of truth) so resume after a
            // restart continues from the next missing chunk.
            state.incoming_files.clone()
        };

        if let Err(e) = crate::persist_incoming_states_snapshot(profile, &snapshot) {
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

            let contact_name = contact_name_for_pubkey(profile, contacts, &msg.from, true);
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

            let group_ref = completed_group
                .as_ref()
                .map(|(i, t)| (i.as_str(), t.as_str()));
            store_file_message(
                profile,
                tui_tx,
                &contact_name,
                group_ref,
                &body,
                msg.timestamp_ms,
                DeliveryStatus::Delivered,
                true,
            )
            .await?;
        }
    }

    Ok(())
}

pub(crate) async fn handle_file_inline(
    profile: &Path,
    tui_tx: &mpsc::Sender<TuiEvent>,
    contacts: &ContactsMap,
    msg: &mut ChatMessage,
) -> Result<()> {
    let (plaintext, verified) = decrypt_and_verify(msg, profile, contacts).unwrap_or_else(|e| {
        tracing::error!(error=%e, "decrypt/verify failed");
        (String::new(), false)
    });

    let contact_name = contact_name_for_pubkey(profile, contacts, &msg.from, verified);
    let inline = match serde_json::from_str::<FileInlinePayload>(&plaintext) {
        Ok(inline) => inline,
        Err(e) => {
            tracing::error!(error=%e, plaintext_len=plaintext.len(), "invalid file_inline payload");
            let body = format!("[file received failed: invalid inline payload: {e}]");
            store_message(
                profile,
                "in",
                &contact_name,
                "",
                &body,
                msg.timestamp_ms,
                DeliveryStatus::Failed,
            )?;
            let _ = tui_tx
                .send(TuiEvent::InboundMessage {
                    contact: contact_name,
                    body,
                    timestamp_ms: msg.timestamp_ms,
                    verified,
                })
                .await;
            return Ok(());
        }
    };

    {
        tracing::info!(name=%inline.name, size=%inline.size, "file_inline received");
        let downloads_dir = profile.join("downloads");
        if let Err(e) = std::fs::create_dir_all(&downloads_dir) {
            tracing::error!(error=%e, "failed to create downloads dir");
        }

        let mut body = format!("[file received failed: {}]", inline.name);
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
                tracing::info!(path=%out_path.display(), "writing received file");
                match write_file_atomically(&out_path, &data) {
                    Ok(_) => {
                        tracing::info!(path=%out_path.display(), "file written successfully");
                        body = format!("[file received: {}]", out_path.display());
                    }
                    Err(e) => {
                        tracing::error!(error=%e, path=%out_path.display(), "file write failed");
                        body = format!("[file write failed: {e}]");
                    }
                }
            } else {
                tracing::warn!(expected=%inline.hash, actual=%actual_hash, "file hash mismatch");
                body = format!("[file hash mismatch: {}]", inline.name);
            }
        } else {
            tracing::error!("base64 decode failed for file_inline");
        }

        let group = inline
            .group_id
            .as_deref()
            .filter(|id| !id.is_empty())
            .map(|id| (id, inline.group_title.as_deref().unwrap_or(id)));
        store_file_message(
            profile,
            tui_tx,
            &contact_name,
            group,
            &body,
            msg.timestamp_ms,
            DeliveryStatus::Delivered,
            verified,
        )
        .await?;
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
            let key = crate::ack_key(&ack.hash, ack.chunk_index);
            if let Ok(mut set) = crate::file_ack_set().lock() {
                set.insert(key);
            }
            accepted = true;
        }
    }
    let contact_name = contact_name_for_pubkey(profile, contacts, &msg.from, verified);
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
fn contact_name_for_pubkey(
    profile: &Path,
    contacts: &ContactsMap,
    pubkey: &str,
    verified: bool,
) -> String {
    if let Some(contact) = contacts.values().find(|c| c.pubkey_b64 == pubkey) {
        return contact.name.clone();
    }

    if verified {
        // decrypt_and_verify can trust-on-first-contact and persist a pending
        // contact while this handler still holds the pre-decrypt contacts
        // snapshot. Reload before choosing the storage/display name; otherwise
        // the first message lands under "verified-peer" and disappears when the
        // user accepts the newly created contact.
        if let Ok(fresh_contacts) = crate::load_contacts(profile) {
            if let Some(contact) = fresh_contacts.values().find(|c| c.pubkey_b64 == pubkey) {
                return contact.name.clone();
            }
        }
        return "verified-peer".into();
    }

    pubkey.to_string()
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
    use super::{
        ack_is_acceptable, body_for_inbound_display, contact_name_for_pubkey, write_file_atomically,
    };
    use crate::{save_contacts, ContactFile, ContactsMap, FileAckPayload};

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

    #[test]
    fn verified_unknown_contact_name_uses_autodiscovered_pending_contact() {
        let dir = tempfile::tempdir().unwrap();
        let pubkey = "sender-pubkey".to_string();
        let mut persisted = ContactsMap::new();
        persisted.insert(
            "alice".to_string(),
            ContactFile {
                name: "alice".to_string(),
                onion: "alice.onion".to_string(),
                pubkey_b64: pubkey.clone(),
                x25519_pubkey_b64: Some("x25519".to_string()),
                pending: true,
                blocked: false,
            },
        );
        save_contacts(dir.path(), &persisted).unwrap();

        let stale_snapshot = ContactsMap::new();
        assert_eq!(
            contact_name_for_pubkey(dir.path(), &stale_snapshot, &pubkey, true),
            "alice"
        );
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

        record_inbound_chat_plaintext(dir.path(), &tx, "alice", &payload, 123, true, None)
            .await
            .unwrap();

        let groups = load_groups(dir.path()).unwrap();
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].id, "g-ops");
        assert_eq!(groups[0].title, "Ops");
        assert!(groups[0].members.iter().any(|m| m.contact == "alice"));
        assert!(groups[0].members.iter().any(|m| m.contact == "bob"));
        // Unknown sender is added as a stub contact for group membership tracking.
        assert!(groups[0]
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

        record_inbound_chat_plaintext(dir.path(), &tx, "stranger", &payload, 456, false, None)
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

async fn handle_group_leave(
    profile: &Path,
    tui_tx: &mpsc::Sender<TuiEvent>,
    contacts: &ContactsMap,
    msg: &mut ChatMessage,
) -> Result<()> {
    let (plaintext, verified) = decrypt_and_verify(msg, profile, contacts).unwrap_or_else(|e| {
        tracing::error!(error=%e, "decrypt/verify failed");
        (String::new(), false)
    });
    let contact_name = contact_name_for_pubkey(profile, contacts, &msg.from, verified);
    if let Ok(payload) = serde_json::from_str::<crate::GroupLeavePayload>(&plaintext) {
        let display = format!("[{} left group]", contact_name);
        let status = if verified {
            DeliveryStatus::Delivered
        } else {
            DeliveryStatus::Failed
        };
        store_message_for_conversation(
            profile,
            "in",
            &contact_name,
            "",
            &display,
            msg.timestamp_ms,
            status,
            "group",
            &payload.group_id,
        )?;
        let _ = tui_tx
            .send(TuiEvent::InboundGroupMessage {
                group_id: payload.group_id,
                group_title: payload.group_title,
                contact: contact_name,
                body: display,
                timestamp_ms: msg.timestamp_ms,
                verified,
            })
            .await;
    }
    Ok(())
}

async fn handle_group_deleted(
    profile: &Path,
    tui_tx: &mpsc::Sender<TuiEvent>,
    contacts: &ContactsMap,
    msg: &mut ChatMessage,
) -> Result<()> {
    let (plaintext, verified) = decrypt_and_verify(msg, profile, contacts).unwrap_or_else(|e| {
        tracing::error!(error=%e, "decrypt/verify failed");
        (String::new(), false)
    });
    let contact_name = contact_name_for_pubkey(profile, contacts, &msg.from, verified);
    if let Ok(payload) = serde_json::from_str::<crate::GroupDeletePayload>(&plaintext) {
        let display = format!("[group deleted by {}]", contact_name);
        let status = if verified {
            DeliveryStatus::Delivered
        } else {
            DeliveryStatus::Failed
        };
        store_message_for_conversation(
            profile,
            "in",
            &contact_name,
            "",
            &display,
            msg.timestamp_ms,
            status,
            "group",
            &payload.group_id,
        )?;
        let _ = tui_tx
            .send(TuiEvent::InboundGroupMessage {
                group_id: payload.group_id,
                group_title: payload.group_title,
                contact: contact_name,
                body: display,
                timestamp_ms: msg.timestamp_ms,
                verified,
            })
            .await;
    }
    Ok(())
}
