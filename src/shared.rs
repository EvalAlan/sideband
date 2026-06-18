#![allow(dead_code)]

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};

// ── Types ──────────────────────────────────────────────────────────────────

pub type ContactsMap = HashMap<String, ContactFile>;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GroupMember {
    pub(crate) contact: String,
    pub(crate) role: String,
    pub(crate) added_at_ms: i64,
}

pub struct ContactFile {
    name: String,
    onion: String,
    pubkey_b64: String,
    /// X25519 public key for encrypting messages to this contact.
    x25519_pubkey_b64: Option<String>,
}

pub struct ChatMessage {
    v: u32,
    r#type: String,
    from: String,
    #[serde(default)]
    sender_name: String,
    #[serde(default)]
    sender_onion: String,
    #[serde(default)]
    sender_x25519_pubkey_b64: String,
    timestamp_ms: u128,
    body: String,
    sig_b64: String,
    /// v2+: hex-encoded nonce(12) + ciphertext.  Empty for v1/v3 messages.
    enc_body: String,
    /// v3+: Double Ratchet header (dh_pub | send_n | prev_send_n), base64.
    #[serde(default)]
    ratchet_header_b64: String,
    /// v3+: Ratchet message nonce, hex.
    #[serde(default)]
    ratchet_nonce_hex: String,
    /// v3+: Ratchet ciphertext, hex.
    #[serde(default)]
    ratchet_ct_hex: String,
}

pub struct FileOfferPayload {
    name: String,
    size: usize,
    hash: String,
    total_chunks: usize,
}

pub struct FileChunkPayload {
    name: String,
    hash: String,
    chunk_index: usize,
    total_chunks: usize,
    data_b64: String,
}

pub struct FileAckPayload {
    hash: String,
    chunk_index: usize,
    total_chunks: usize,
    status: String,
}

pub struct GroupInfo {
    pub(crate) id: String,
    pub(crate) title: String,
    pub(crate) created_at_ms: i64,
    pub(crate) updated_at_ms: i64,
    pub(crate) members: Vec<GroupMember>,
}

pub struct GroupMember {
    pub(crate) contact: String,
    pub(crate) role: String,
    pub(crate) added_at_ms: i64,
}

// ── Functions ──────────────────────────────────────────────────────────────

#![allow(dead_code)]

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};

pub fn expand_home(path: &Path) -> PathBuf {
    if path == Path::new("~") {
        if let Some(home) = std::env::var_os("HOME") {
            return PathBuf::from(home);
        }
    }

    if let Ok(rest) = path.strip_prefix("~/") {
        if let Some(home) = std::env::var_os("HOME") {
            return PathBuf::from(home).join(rest);
        }
    }

    path.to_path_buf()
}

fn contacts_path(profile: &Path) -> PathBuf {
    profile.join("contacts.toml")
}

pub fn load_contacts(profile: &Path) -> Result<ContactsMap> {
    let p = contacts_path(profile);
    if !p.exists() {
        return Ok(HashMap::new());
    }
    let text = fs::read_to_string(&p).with_context(|| format!("read {}", p.display()))?;
    let map: ContactsMap = toml::from_str(&text)?;
    Ok(map)
}

pub fn save_contacts(profile: &Path, contacts: &ContactsMap) -> Result<()> {
    let p = contacts_path(profile);
    fs::write(&p, toml::to_string_pretty(contacts)?)
        .with_context(|| format!("write {}", p.display()))?;
    Ok(())
}

pub fn validate_contact_fields(onion: &str, pubkey_b64: &str, x25519_pubkey_b64: &str) -> Result<()> {
    onion
        .parse::<tor_hsservice::HsId>()
        .map_err(|e| anyhow!("invalid v3 onion address '{}': {}", onion, e))?;
    let raw = B64
        .decode(pubkey_b64.as_bytes())
        .context("decode pubkey base64")?;
    if raw.len() != 32 {
        return Err(anyhow!("pubkey must decode to 32 bytes, got {}", raw.len()));
    }
    let raw_x = B64
        .decode(x25519_pubkey_b64.as_bytes())
        .context("decode x25519 pubkey base64")?;
    if raw_x.len() != 32 {
        return Err(anyhow!(
            "x25519 pubkey must decode to 32 bytes, got {}",
            raw_x.len()
        ));
    }
    Ok(())
}

pub fn contact_add(
    profile: &Path,
    name: &str,
    onion: &str,
    pubkey_b64: &str,
    x25519_pubkey_b64: &str,
) -> Result<()> {
    validate_contact_fields(onion, pubkey_b64, x25519_pubkey_b64)?;

    let mut contacts = load_contacts(profile)?;
    contacts.insert(
        name.to_string(),
        ContactFile {
            name: name.to_string(),
            onion: onion.to_string(),
            pubkey_b64: pubkey_b64.to_string(),
            x25519_pubkey_b64: Some(x25519_pubkey_b64.to_string()),
        },
    );
    save_contacts(profile, &contacts)?;
    println!("contact '{name}' added");
    Ok(())
}

pub fn contact_delete(profile: &Path, name: &str) -> Result<bool> {
    let mut contacts = load_contacts(profile)?;
    if contacts.remove(name).is_some() {
        save_contacts(profile, &contacts)?;
        Ok(true)
    } else {
        Ok(false)
    }
}

pub fn identity_path(profile: &Path) -> PathBuf {
    profile.join("identity.toml")
}

pub fn default_display_name(profile: &Path) -> String {
    profile
        .file_name()
        .and_then(|n| n.to_str())
        .map(|name| name.trim_start_matches('.'))
        .filter(|name| !name.is_empty())
        .unwrap_or("sideband")
        .to_string()
}

pub fn load_display_name(profile: &Path) -> Result<String> {
    let mut id = load_identity(profile)?;
    if id.display_name.trim().is_empty() {
        id.display_name = default_display_name(profile);
        save_identity(profile, &id)?;
    }
    Ok(id.display_name)
}

pub fn init_profile_with_name(profile: &Path, display_name: &str) -> Result<()> {
    fs::create_dir_all(profile).context("create profile dir")?;

    let signing = SigningKey::generate(&mut OsRng);
    let x25519_secret = StaticSecret::random_from_rng(OsRng);
    let id_file = IdentityFile {
        secret_key_b64: B64.encode(signing.to_bytes()),
        display_name: display_name.to_string(),
        x25519_secret_b64: B64.encode(x25519_secret.to_bytes()),
    };
    let identity_path = identity_path(profile);
    fs::write(&identity_path, toml::to_string_pretty(&id_file)?).context("write identity")?;

    fs::create_dir_all(profile.join("arti_state"))?;

    // SQLite schema (no-op if already exists)
    init_db(profile)?;

    info!(profile=%profile.display(), name=%display_name, "profile initialized");
    Ok(())
}

pub fn resolve_to(profile: &Path, to: &str) -> Result<String> {
    if to.ends_with(".onion") {
        return Ok(to.to_string());
    }
    let contacts = load_contacts(profile)?;
    match contacts.get(to) {
        Some(c) => Ok(c.onion.clone()),
        None => Err(anyhow!(
            "unknown contact '{to}'. Add with: sideband contact add --name {to} --onion <addr>.onion --pubkey <b64>"
        )),
    }
}

pub fn load_history(
    profile: &Path,
    contact_filter: Option<&str>,
    limit: usize,
) -> Result<Vec<HistoryRow>> {
    let conn = init_db(profile)?;

    if let Some(c) = contact_filter {
        let mut stmt = conn.prepare(
            "SELECT id, direction, contact, onion, body, timestamp_ms, status, created_at, conversation_kind, conversation_id
             FROM messages
             WHERE conversation_kind = 'contact' AND contact = ?1
               AND body NOT LIKE '%group_message%'
             ORDER BY timestamp_ms DESC LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![c, limit as i64], history_row_from_sql)?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    } else {
        let mut stmt = conn.prepare(
            "SELECT id, direction, contact, onion, body, timestamp_ms, status, created_at, conversation_kind, conversation_id
             FROM messages
             ORDER BY timestamp_ms DESC LIMIT ?1",
        )?;
        let rows = stmt.query_map(params![limit as i64], history_row_from_sql)?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }
}

pub fn list_transfers(profile: &Path) -> Result<Vec<String>> {
    let mut rows = Vec::new();
    let conn = init_db(profile)?;

    let mut out_stmt = conn.prepare(
        "SELECT hash, contact_name, file_name, next_chunk_index, total_chunks
         FROM outbound_transfers ORDER BY updated_at DESC",
    )?;
    let out_rows = out_stmt.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, i64>(3)? as usize,
            r.get::<_, i64>(4)? as usize,
        ))
    })?;
    for row in out_rows {
        let (hash, contact_name, file_name, next_chunk_index, total_chunks) = row?;
        rows.push(format!(
            "outbound {} -> {} chunk {}/{} file={}",
            hash, contact_name, next_chunk_index, total_chunks, file_name
        ));
    }

    if let Ok(map) = incoming_files_map().lock() {
        for (k, st) in map.iter() {
            let have = st.chunks.iter().filter(|c| c.is_some()).count();
            rows.push(format!(
                "incoming {} chunks {}/{}",
                k, have, st.total_chunks
            ));
        }
    }

    rows.sort();
    Ok(rows)
}

pub fn cancel_outbound_transfer(profile: &Path, hash: &str) -> Result<bool> {
    let conn = init_db(profile)?;
    let affected = conn.execute(
        "DELETE FROM outbound_transfers WHERE hash = ?1",
        params![hash],
    )?;
    Ok(affected > 0)
}

pub fn outbound_transfer_target(
    profile: &Path,
    hash: &str,
) -> Result<Option<(String, String)>> {
    let Some(st) = load_outbound_state(profile, hash)? else {
        return Ok(None);
    };
    Ok(Some((st.contact_name, st.file_path)))
}

pub async fn send_file(
    profile: &Path,
    contact_name: &str,
    file_path: &str,
    _reuse_socks_port: Option<u16>,
    tor_client: Arc<TorClient<PreferredRuntime>>,
) -> Result<()> {
    use sha2::Digest;
    use std::io::Read;

    let path = std::path::Path::new(file_path);
    if !path.exists() {
        return Err(anyhow!("file not found: {}", file_path));
    }
    let file_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("unknown")
        .to_string();
    let mut file = std::fs::File::open(path)?;
    let mut content = Vec::new();
    file.read_to_end(&mut content)?;
    let total_size = content.len();
    let hash = {
        let mut h = sha2::Sha256::new();
        h.update(&content);
        format!("{:x}", h.finalize())
    };
    let total_chunks = total_size.div_ceil(FILE_CHUNK_SIZE);

    let onion = crate::resolve_to(profile, contact_name)?;

    // Fast path for tiny files: send as one inline payload over a single message.
    // This avoids chunk/ack round-trips that are fragile over hidden-service circuits.
    if total_size <= FILE_INLINE_MAX_SIZE {
        let inline = FileInlinePayload {
            name: file_name.clone(),
            size: total_size,
            hash: hash.clone(),
            data_b64: B64.encode(&content),
        };
        let inline_json = serde_json::to_string(&inline)?;

        tracing::info!(contact=%contact_name, name=%file_name, size=total_size, "sending file_inline");
        let mut sent = false;
        let mut last_error = String::new();
        for attempt in 1..=4 {
            match send_typed_message(
                profile,
                &onion,
                contact_name,
                "file_inline",
                &inline_json,
                Arc::clone(&tor_client),
            )
            .await
            {
                Ok(()) => {
                    sent = true;
                    break;
                }
                Err(e) => {
                    last_error = e.to_string();
                    warn!(attempt, error=%last_error, "file_inline send failed");
                    if attempt < 4 {
                        tokio::time::sleep(Duration::from_secs(3)).await;
                    }
                }
            }
        }

        if !sent {
            return Err(anyhow!("file_inline send failed to {contact_name}: {last_error}"));
        }

        let timestamp_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_millis();
        tracing::info!(contact=%contact_name, name=%file_name, size=total_size, "file_inline sent OK");
        crate::store_message(
            profile,
            "out",
            contact_name,
            &onion,
            &format!("[file sent: {} ({} bytes, inline)]", file_path, total_size),
            timestamp_ms,
            crate::DeliveryStatus::Sent,
        )?;

        drop(tor_client);
        return Ok(());
    }

    let mut outbound_state =
        load_outbound_state(profile, &hash)?.unwrap_or(OutboundTransferState {
            contact_name: contact_name.to_string(),
            onion: onion.clone(),
            file_name: file_name.clone(),
            file_path: file_path.to_string(),
            hash: hash.clone(),
            total_size,
            total_chunks,
            next_chunk_index: 0,
        });

    // Ensure state reflects current invocation paths.
    outbound_state.contact_name = contact_name.to_string();
    outbound_state.onion = onion.clone();
    outbound_state.file_name = file_name.clone();
    outbound_state.file_path = file_path.to_string();
    outbound_state.total_size = total_size;
    outbound_state.total_chunks = total_chunks;
    persist_outbound_state(profile, &outbound_state)?;

    if outbound_state.next_chunk_index == 0 {
        let offer = FileOfferPayload {
            name: file_name.clone(),
            size: total_size,
            hash: hash.clone(),
            total_chunks,
        };
        let offer_json = serde_json::to_string(&offer)?;

        let mut offer_sent = false;
        for attempt in 1..=3 {
            match send_typed_message(
                profile,
                &onion,
                contact_name,
                "file_offer",
                &offer_json,
                Arc::clone(&tor_client),
            )
            .await
            {
                Ok(()) => {
                    offer_sent = true;
                    break;
                }
                Err(e) => {
                    warn!(attempt, error=%e, "file offer send failed");
                    if attempt < 3 {
                        tokio::time::sleep(Duration::from_secs(3)).await;
                    }
                }
            }
        }
        if !offer_sent {
            return Err(anyhow!(
                "file transfer failed sending offer to {}",
                contact_name
            ));
        }

        // Give the receiver a short window to persist offer state before the first chunk.
        // Without this, first chunk can race with offer handling on slower HS paths.
        tokio::time::sleep(Duration::from_millis(1200)).await;
    }

    for chunk_index in outbound_state.next_chunk_index..total_chunks {
        let start = chunk_index * FILE_CHUNK_SIZE;
        let end = ((chunk_index + 1) * FILE_CHUNK_SIZE).min(total_size);
        let payload = FileChunkPayload {
            name: file_name.clone(),
            hash: hash.clone(),
            chunk_index,
            total_chunks,
            data_b64: B64.encode(&content[start..end]),
        };
        let chunk_json = serde_json::to_string(&payload)?;

        let mut delivered = false;
        let max_attempts = 4;
        let payload_bytes = chunk_json.len();
        for attempt in 1..=max_attempts {
            tracing::info!(
                chunk_index,
                total_chunks,
                attempt,
                max_attempts,
                payload_bytes,
                "sending file chunk"
            );

            let send_result = send_typed_message(
                profile,
                &onion,
                contact_name,
                "file_chunk",
                &chunk_json,
                Arc::clone(&tor_client),
            )
            .await;

            if let Err(e) = send_result {
                warn!(chunk_index, attempt, payload_bytes, error=%e, "file chunk send failed");
                if attempt < max_attempts {
                    let jitter_ms = 250
                        + ((SystemTime::now()
                            .duration_since(UNIX_EPOCH)
                            .map(|d| d.as_millis() as u64)
                            .unwrap_or(0)
                            + chunk_index as u64 * 97
                            + attempt as u64 * 131)
                            % 900);
                    let backoff_secs = attempt as u64 * 2;
                    tokio::time::sleep(
                        Duration::from_secs(backoff_secs) + Duration::from_millis(jitter_ms),
                    )
                    .await;
                    continue;
                }
                if e.to_string().contains("timed out") {
                    return Err(anyhow!(
                        "chunk_connect_timeout chunk={}/{} payload={}B attempts={} cause={}",
                        chunk_index + 1,
                        total_chunks,
                        payload_bytes,
                        max_attempts,
                        e
                    ));
                }
                return Err(anyhow!(
                    "chunk_send_failed chunk={}/{} payload={}B attempts={} cause={}",
                    chunk_index + 1,
                    total_chunks,
                    payload_bytes,
                    max_attempts,
                    e
                ));
            }

            if wait_for_file_ack(&hash, chunk_index, std::time::Duration::from_secs(20)).await {
                delivered = true;
                break;
            }

            if attempt < max_attempts {
                warn!(
                    chunk_index,
                    attempt, payload_bytes, "file chunk ack timeout; retrying"
                );
            }
        }

        if !delivered {
            return Err(anyhow!(
                "file transfer ack_timeout hash={} chunk={}/{} payload={}B attempts={}",
                hash,
                chunk_index + 1,
                total_chunks,
                payload_bytes,
                max_attempts
            ));
        }

        outbound_state.next_chunk_index = chunk_index + 1;
        persist_outbound_state(profile, &outbound_state)?;
    }

    let timestamp_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_millis();
    crate::store_message(
        profile,
        "out",
        contact_name,
        &onion,
        &format!(
            "[file sent: {} ({} bytes, {} chunks)]",
            file_path, total_size, total_chunks
        ),
        timestamp_ms,
        crate::DeliveryStatus::Sent,
    )?;

    clear_outbound_state(profile, &hash);
    drop(tor_client);
    Ok(())
}

pub async fn send(
    profile: &Path,
    to: &str,
    message: &str,
    contact_hint: &str,
    reuse_socks_port: Option<u16>,
    tor_client: Arc<TorClient<PreferredRuntime>>,
    force_static: bool,
) -> Result<()> {
    send_in_conversation(
        profile,
        to,
        message,
        contact_hint,
        reuse_socks_port,
        tor_client,
        force_static,
        "contact",
        contact_hint,
        true,
    )
    .await
}

pub(crate) fn store_message(
    profile: &Path,
    direction: &str,
    contact: &str,
    onion: &str,
    body: &str,
    timestamp_ms: u128,
    status: DeliveryStatus,
) -> Result<()> {
    store_message_for_conversation(
        profile,
        direction,
        contact,
        onion,
        body,
        timestamp_ms,
        status,
        "contact",
        contact,
    )
}

pub(crate) fn store_message_for_conversation(
    profile: &Path,
    direction: &str,
    contact: &str,
    onion: &str,
    body: &str,
    timestamp_ms: u128,
    status: DeliveryStatus,
    conversation_kind: &str,
    conversation_id: &str,
) -> Result<()> {
    let conn = init_db(profile)?;
    conn.execute(
        "INSERT INTO messages (direction, contact, onion, body, timestamp_ms, status, conversation_kind, conversation_id)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            direction,
            contact,
            onion,
            body,
            timestamp_ms as i64,
            status.as_i64(),
            conversation_kind,
            conversation_id,
        ],
    )?;
    Ok(())
}
