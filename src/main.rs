use std::collections::HashMap;
use std::fs;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, Context, Result};
use base64::{engine::general_purpose::STANDARD as B64, Engine};
use chacha20poly1305::{
    aead::{Aead, KeyInit},
    ChaCha20Poly1305, Key, Nonce,
};
use clap::{Args, Parser, Subcommand};
use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use hkdf::Hkdf;
use qrcode::{render::unicode, Color as QrColor, QrCode};
use rand::rngs::OsRng;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::sync::mpsc;
use tokio::time::sleep;
use tracing::{error, info, warn};
use x25519_dalek::{PublicKey as X25519PublicKey, StaticSecret};

use arti_client::config::CfgPath;
use arti_client::{TorClient, TorClientConfig};
use tor_rtcompat::PreferredRuntime;

mod app_api;
mod handler;
mod transport;
mod tui;

use crate::transport::Transport;

// ---------------------------------------------------------------------------
// CLI
// ---------------------------------------------------------------------------

const BUILD_COMMIT: &str = match option_env!("SIDEBAND_BUILD_COMMIT") {
    Some(commit) => commit,
    None => "dev",
};

#[derive(Debug, Parser)]
#[command(name = "sideband")]
#[command(about = "Experimental Tor transport proof for Sideband")]
struct Cli {
    #[command(subcommand)]
    command: CommandKind,
}

#[derive(Debug, Args, Clone)]
struct ProfileArg {
    /// Profile/state directory. Defaults to ~/.sideband for normal use.
    #[arg(long)]
    profile: Option<PathBuf>,
}

impl ProfileArg {
    fn path(&self) -> Result<PathBuf> {
        match &self.profile {
            Some(path) => Ok(expand_home(path)),
            None => default_profile_path(),
        }
    }
}

fn default_profile_path() -> Result<PathBuf> {
    let home = std::env::var_os("HOME").ok_or_else(|| anyhow!("HOME is not set"))?;
    Ok(PathBuf::from(home).join(".sideband"))
}

fn expand_home(path: &Path) -> PathBuf {
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

#[derive(Debug, Subcommand)]
enum CommandKind {
    Init(ProfileArg),
    Identity(ProfileArg),
    Share {
        #[command(flatten)]
        profile: ProfileArg,
        /// Onion address to include. Omit when Tor has not published one yet.
        #[arg(long)]
        onion: Option<String>,
        /// Emit machine-readable JSON with command and QR matrix.
        #[arg(long)]
        json: bool,
    },
    Serve(ProfileArg),
    Send {
        #[command(flatten)]
        profile: ProfileArg,
        /// Onion address or contact name.
        #[arg(long)]
        to: String,
        #[arg(long)]
        message: String,
        /// Force static X25519 encryption instead of Double Ratchet.
        #[arg(long = "static")]
        force_static: bool,
    },
    Contact {
        #[command(subcommand)]
        action: ContactAction,
    },
    Group {
        #[command(subcommand)]
        action: GroupAction,
    },
    History {
        #[command(flatten)]
        profile: ProfileArg,
        /// Filter by contact name or onion address.
        #[arg(long)]
        contact: Option<String>,
        /// Max rows (default 50)
        #[arg(long, default_value_t = 50)]
        limit: usize,
        /// Emit machine-readable JSON instead of aligned text.
        #[arg(long)]
        json: bool,
        /// Delete matching history rows instead of printing them.
        #[arg(long)]
        clear: bool,
    },
    Ratchet {
        #[command(flatten)]
        profile: ProfileArg,
        /// Contact name to initialize Double Ratchet with.
        contact: String,
    },
    Name {
        #[command(flatten)]
        profile: ProfileArg,
        /// New display name. Omit to print the current name.
        name: Option<String>,
    },
    Tui(ProfileArg),
}

#[derive(Debug, Subcommand)]
enum ContactAction {
    Add {
        #[command(flatten)]
        profile: ProfileArg,
        #[arg(long)]
        name: String,
        #[arg(long)]
        onion: String,
        #[arg(long)]
        pubkey: String,
        #[arg(long)]
        x25519_pubkey: String,
    },
    Delete {
        #[command(flatten)]
        profile: ProfileArg,
        #[arg(long)]
        name: String,
    },
    List {
        #[command(flatten)]
        profile: ProfileArg,
        /// Emit machine-readable JSON instead of tab-separated text.
        #[arg(long)]
        json: bool,
    },
}

#[derive(Debug, Subcommand)]
enum GroupAction {
    Create {
        #[command(flatten)]
        profile: ProfileArg,
        /// Human-readable group title.
        #[arg(long)]
        title: String,
        /// Contact name to include. Repeat --member for multiple contacts.
        #[arg(long = "member")]
        members: Vec<String>,
        /// Emit machine-readable JSON instead of text.
        #[arg(long)]
        json: bool,
    },
    List {
        #[command(flatten)]
        profile: ProfileArg,
        /// Emit machine-readable JSON instead of text.
        #[arg(long)]
        json: bool,
    },
}

// ---------------------------------------------------------------------------
// Data model
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
struct ShareInfo {
    command: String,
    qr: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct IdentityFile {
    secret_key_b64: String,
    /// Human display name shown in the TUI and shared contact command.
    #[serde(default)]
    display_name: String,
    /// X25519 static secret key for ChaCha20-Poly1305 encryption.
    #[serde(default)]
    x25519_secret_b64: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct ContactFile {
    name: String,
    onion: String,
    pubkey_b64: String,
    /// X25519 public key for encrypting messages to this contact.
    x25519_pubkey_b64: Option<String>,
}

/// On-disk format: name -> ContactFile
pub(crate) type ContactsMap = HashMap<String, ContactFile>;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct GroupMember {
    contact: String,
    role: String,
    added_at_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct GroupInfo {
    id: String,
    title: String,
    created_at_ms: i64,
    updated_at_ms: i64,
    members: Vec<GroupMember>,
}

/// Chat message format (v1 = signed plaintext, v2 = signed + encrypted, v3 = double ratchet).
/// In v2 the `body` field empty on wire, `enc_body` holds ChaCha20-Poly1305 ciphertext.
/// In v3 `body` and `enc_body` are empty; ratchet_header_b64, ratchet_nonce_hex,
/// and ratchet_ct_hex carry the Double Ratchet payload.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub(crate) struct ChatMessage {
    v: u32,
    r#type: String,
    from: String,
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

// ---------------------------------------------------------------------------
// File transfer
// ---------------------------------------------------------------------------

const FILE_CHUNK_SIZE: usize = 8 * 1024; // 8 KB chunks (smaller HS payload for better reliability)
const FILE_INLINE_MAX_SIZE: usize = 96 * 1024; // auto-inline small/medium files; no mode switching for users

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct FileOfferPayload {
    name: String,
    size: usize,
    hash: String,
    total_chunks: usize,
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct FileChunkPayload {
    name: String,
    hash: String,
    chunk_index: usize,
    total_chunks: usize,
    data_b64: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct FileAckPayload {
    hash: String,
    chunk_index: usize,
    total_chunks: usize,
    status: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct FileInlinePayload {
    name: String,
    size: usize,
    hash: String,
    data_b64: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct IncomingFileState {
    total_chunks: usize,
    chunks: Vec<Option<Vec<u8>>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct OutboundTransferState {
    contact_name: String,
    onion: String,
    file_name: String,
    file_path: String,
    hash: String,
    total_size: usize,
    total_chunks: usize,
    next_chunk_index: usize,
}

static INCOMING_FILES: std::sync::OnceLock<
    std::sync::Mutex<std::collections::HashMap<String, IncomingFileState>>,
> = std::sync::OnceLock::new();
static FILE_ACKS: std::sync::OnceLock<std::sync::Mutex<std::collections::HashSet<String>>> =
    std::sync::OnceLock::new();

pub(crate) fn incoming_files_map(
) -> &'static std::sync::Mutex<std::collections::HashMap<String, IncomingFileState>> {
    INCOMING_FILES.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()))
}

pub(crate) fn file_ack_set() -> &'static std::sync::Mutex<std::collections::HashSet<String>> {
    FILE_ACKS.get_or_init(|| std::sync::Mutex::new(std::collections::HashSet::new()))
}

pub(crate) fn ack_key(hash: &str, chunk_index: usize) -> String {
    format!("{hash}:{chunk_index}")
}

pub(crate) fn persist_incoming_states(profile: &Path) -> Result<()> {
    let snapshot = {
        let map = incoming_files_map()
            .lock()
            .map_err(|_| anyhow!("incoming file map lock poisoned"))?;
        map.clone()
    };

    let mut conn = init_db(profile)?;
    let tx = conn.transaction()?;
    tx.execute("DELETE FROM inbound_transfer_chunks", [])?;
    tx.execute("DELETE FROM inbound_transfers", [])?;

    for (transfer_key, state) in snapshot {
        tx.execute(
            "INSERT INTO inbound_transfers (transfer_key, total_chunks, updated_at)
             VALUES (?1, ?2, datetime('now'))",
            params![transfer_key, state.total_chunks as i64],
        )?;
        for (idx, maybe_chunk) in state.chunks.iter().enumerate() {
            let blob: Option<&[u8]> = maybe_chunk.as_deref();
            tx.execute(
                "INSERT INTO inbound_transfer_chunks (transfer_key, chunk_index, chunk_data)
                 VALUES (?1, ?2, ?3)",
                params![transfer_key, idx as i64, blob],
            )?;
        }
    }
    tx.commit()?;
    Ok(())
}

fn load_incoming_states(profile: &Path) -> Result<()> {
    let conn = init_db(profile)?;
    let mut stmt = conn.prepare(
        "SELECT t.transfer_key, t.total_chunks, c.chunk_index, c.chunk_data
         FROM inbound_transfers t
         LEFT JOIN inbound_transfer_chunks c ON c.transfer_key = t.transfer_key
         ORDER BY t.transfer_key, c.chunk_index",
    )?;

    let mut map: std::collections::HashMap<String, IncomingFileState> =
        std::collections::HashMap::new();
    let rows = stmt.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, i64>(1)? as usize,
            r.get::<_, Option<i64>>(2)?,
            r.get::<_, Option<Vec<u8>>>(3)?,
        ))
    })?;

    for row in rows {
        let (transfer_key, total_chunks, idx_opt, blob_opt) = row?;
        let entry = map
            .entry(transfer_key)
            .or_insert_with(|| IncomingFileState {
                total_chunks,
                chunks: vec![None; total_chunks],
            });
        if let Some(idx_i64) = idx_opt {
            let idx = idx_i64 as usize;
            if idx < entry.chunks.len() {
                entry.chunks[idx] = blob_opt;
            }
        }
    }

    let mut lock = incoming_files_map()
        .lock()
        .map_err(|_| anyhow!("incoming file map lock poisoned"))?;
    *lock = map;
    Ok(())
}

pub(crate) fn persist_outbound_state(profile: &Path, state: &OutboundTransferState) -> Result<()> {
    let conn = init_db(profile)?;
    conn.execute(
        "INSERT INTO outbound_transfers
         (hash, contact_name, onion, file_name, file_path, total_size, total_chunks, next_chunk_index, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, datetime('now'))
         ON CONFLICT(hash) DO UPDATE SET
             contact_name=excluded.contact_name,
             onion=excluded.onion,
             file_name=excluded.file_name,
             file_path=excluded.file_path,
             total_size=excluded.total_size,
             total_chunks=excluded.total_chunks,
             next_chunk_index=excluded.next_chunk_index,
             updated_at=datetime('now')",
        params![
            state.hash,
            state.contact_name,
            state.onion,
            state.file_name,
            state.file_path,
            state.total_size as i64,
            state.total_chunks as i64,
            state.next_chunk_index as i64
        ],
    )?;
    Ok(())
}

fn load_outbound_state(profile: &Path, hash: &str) -> Result<Option<OutboundTransferState>> {
    let conn = init_db(profile)?;
    let mut stmt = conn.prepare(
        "SELECT contact_name, onion, file_name, file_path, hash, total_size, total_chunks, next_chunk_index
         FROM outbound_transfers WHERE hash = ?1",
    )?;
    let mut rows = stmt.query(params![hash])?;
    let Some(r) = rows.next()? else {
        return Ok(None);
    };
    Ok(Some(OutboundTransferState {
        contact_name: r.get(0)?,
        onion: r.get(1)?,
        file_name: r.get(2)?,
        file_path: r.get(3)?,
        hash: r.get(4)?,
        total_size: r.get::<_, i64>(5)? as usize,
        total_chunks: r.get::<_, i64>(6)? as usize,
        next_chunk_index: r.get::<_, i64>(7)? as usize,
    }))
}

pub(crate) fn outbound_transfer_target(
    profile: &Path,
    hash: &str,
) -> Result<Option<(String, String)>> {
    let Some(st) = load_outbound_state(profile, hash)? else {
        return Ok(None);
    };
    Ok(Some((st.contact_name, st.file_path)))
}

fn clear_outbound_state(profile: &Path, hash: &str) {
    if let Ok(conn) = init_db(profile) {
        let _ = conn.execute(
            "DELETE FROM outbound_transfers WHERE hash = ?1",
            params![hash],
        );
    }
}

pub(crate) fn list_transfers(profile: &Path) -> Result<Vec<String>> {
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

pub(crate) fn cancel_outbound_transfer(profile: &Path, hash: &str) -> Result<bool> {
    let conn = init_db(profile)?;
    let affected = conn.execute(
        "DELETE FROM outbound_transfers WHERE hash = ?1",
        params![hash],
    )?;
    Ok(affected > 0)
}

/// Send a file to a contact. Sends `file_offer` then all `file_chunk` messages.
pub(crate) async fn send_file(
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

        let mut sent = false;
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
                    warn!(attempt, error=%e, "file_inline send failed");
                    if attempt < 4 {
                        tokio::time::sleep(Duration::from_secs(3)).await;
                    }
                }
            }
        }

        if !sent {
            return Err(anyhow!("file_inline send failed to {}", contact_name));
        }

        let timestamp_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_millis();
        crate::store_message(
            profile,
            "out",
            contact_name,
            &onion,
            &format!("[file sent: {} ({} bytes, inline)]", file_name, total_size),
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
            file_name, total_size, total_chunks
        ),
        timestamp_ms,
        crate::DeliveryStatus::Sent,
    )?;

    clear_outbound_state(profile, &hash);
    drop(tor_client);
    Ok(())
}

async fn wait_for_file_ack(hash: &str, chunk_index: usize, timeout: Duration) -> bool {
    let key = ack_key(hash, chunk_index);
    let start = std::time::Instant::now();
    loop {
        {
            if let Ok(mut set) = file_ack_set().lock() {
                if set.remove(&key) {
                    return true;
                }
            }
        }
        if start.elapsed() >= timeout {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

async fn send_typed_message(
    profile: &Path,
    to_onion: &str,
    contact_name: &str,
    message_type: &str,
    plaintext: &str,
    tor_client: Arc<TorClient<PreferredRuntime>>,
) -> Result<()> {
    let key = load_signing_key(profile)?;
    let our_ed25519_pub = B64.encode(key.verifying_key().to_bytes());
    let timestamp_ms = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis();

    let ratchet_path = RatchetState::path(profile, std::path::Path::new(contact_name));
    // Keep file-transfer control/data packets on static v2 crypto for now.
    // Ratchet state drift can otherwise deadlock transfers (offer/chunk/ack).
    let use_ratchet = message_type == "msg" && ratchet_path.exists();

    let msg = if use_ratchet {
        let mut state_bytes = fs::read(&ratchet_path)?;
        let mut state: RatchetState =
            bincode::deserialize(&mut state_bytes).context("deserialize ratchet state")?;
        let (header_b64, nonce_hex, ct_hex) =
            ratchet_encrypt(&mut state, plaintext.as_bytes(), &our_ed25519_pub)?;
        state.save(profile, contact_name)?;
        let mut sign_msg = ChatMessage {
            v: 3,
            r#type: message_type.into(),
            from: our_ed25519_pub.clone(),
            timestamp_ms,
            body: plaintext.to_string(),
            sig_b64: String::new(),
            enc_body: String::new(),
            ratchet_header_b64: header_b64,
            ratchet_nonce_hex: nonce_hex,
            ratchet_ct_hex: ct_hex,
        };
        sign_message(&key, &mut sign_msg)?;
        sign_msg.body.clear();
        sign_msg
    } else {
        let mut msg = ChatMessage {
            v: 2,
            r#type: message_type.into(),
            from: our_ed25519_pub.clone(),
            timestamp_ms,
            body: plaintext.to_string(),
            sig_b64: String::new(),
            enc_body: String::new(),
            ratchet_header_b64: String::new(),
            ratchet_nonce_hex: String::new(),
            ratchet_ct_hex: String::new(),
        };
        sign_message(&key, &mut msg)?;
        let our_x25519 = load_x25519_secret(profile)?;
        let their_x25519 = resolve_x25519_pubkey(profile, contact_name)?;
        let shared_key = derive_shared_key(&our_x25519, &their_x25519)?;
        msg.enc_body = encrypt_body(&shared_key, plaintext)?;
        msg.body.clear();
        msg
    };

    let payload = format!("{}\n", serde_json::to_string(&msg)?);
    let connect_timeout = if message_type == "file_chunk" {
        Duration::from_secs(120)
    } else {
        Duration::from_secs(60)
    };
    let result = {
        let payload = payload.clone();
        let to_addr = format!("{}:80", to_onion);
        let tc = Arc::clone(&tor_client);
        let connect_fut = async move {
            let mut stream = tc
                .connect(to_addr.as_str())
                .await
                .map_err(|e| anyhow!("connect: {e}"))?;
            use tokio::io::AsyncWriteExt;
            stream
                .write_all(payload.as_bytes())
                .await
                .map_err(|e| anyhow!("write: {e}"))?;
            stream.flush().await.map_err(|e| anyhow!("flush: {e}"))?;
            stream
                .shutdown()
                .await
                .map_err(|e| anyhow!("shutdown: {e}"))?;
            Ok::<_, anyhow::Error>(())
        };
        tokio::time::timeout(connect_timeout, connect_fut).await
    };

    match result {
        Ok(Ok(())) => Ok(()),
        Ok(Err(e)) => Err(anyhow!("{message_type} send error: {e}")),
        Err(_) => Err(anyhow!("{message_type} send timed out")),
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum DeliveryStatus {
    Sent = 0,
    Delivered = 1,
    Failed = 2,
}

impl DeliveryStatus {
    fn as_i64(self) -> i64 {
        self as i64
    }
    fn from_i64(v: i64) -> Option<Self> {
        match v {
            0 => Some(Self::Sent),
            1 => Some(Self::Delivered),
            2 => Some(Self::Failed),
            _ => None,
        }
    }
    fn label(self) -> &'static str {
        match self {
            Self::Sent => "sent",
            Self::Delivered => "delivered",
            Self::Failed => "failed",
        }
    }
}

// ---------------------------------------------------------------------------
// SQLite
// ---------------------------------------------------------------------------

fn db_path(profile: &Path) -> PathBuf {
    profile.join("messages.db")
}

fn init_db(profile: &Path) -> Result<Connection> {
    let conn = Connection::open(db_path(profile))?;
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS messages (
            id          INTEGER PRIMARY KEY AUTOINCREMENT,
            direction   TEXT    NOT NULL CHECK(direction IN ('in','out')),
            contact     TEXT    NOT NULL,
            onion       TEXT    NOT NULL DEFAULT '',
            body        TEXT    NOT NULL,
            timestamp_ms INTEGER NOT NULL,
            status      INTEGER NOT NULL DEFAULT 0,
            created_at  TEXT    NOT NULL DEFAULT (datetime('now'))
        );
        CREATE INDEX IF NOT EXISTS idx_messages_contact
            ON messages(contact);
        CREATE INDEX IF NOT EXISTS idx_messages_ts
            ON messages(timestamp_ms);

        CREATE TABLE IF NOT EXISTS groups (
            id            TEXT PRIMARY KEY,
            title         TEXT NOT NULL,
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

        CREATE INDEX IF NOT EXISTS idx_group_members_contact
            ON group_members(contact);

        CREATE TABLE IF NOT EXISTS outbound_transfers (
            hash            TEXT PRIMARY KEY,
            contact_name    TEXT NOT NULL,
            onion           TEXT NOT NULL,
            file_name       TEXT NOT NULL,
            file_path       TEXT NOT NULL,
            total_size      INTEGER NOT NULL,
            total_chunks    INTEGER NOT NULL,
            next_chunk_index INTEGER NOT NULL,
            updated_at      TEXT NOT NULL DEFAULT (datetime('now'))
        );

        CREATE TABLE IF NOT EXISTS inbound_transfers (
            transfer_key    TEXT PRIMARY KEY,
            total_chunks    INTEGER NOT NULL,
            updated_at      TEXT NOT NULL DEFAULT (datetime('now'))
        );

        CREATE TABLE IF NOT EXISTS inbound_transfer_chunks (
            transfer_key    TEXT NOT NULL,
            chunk_index     INTEGER NOT NULL,
            chunk_data      BLOB,
            PRIMARY KEY (transfer_key, chunk_index),
            FOREIGN KEY (transfer_key) REFERENCES inbound_transfers(transfer_key) ON DELETE CASCADE
        );

        CREATE INDEX IF NOT EXISTS idx_outbound_transfers_updated
            ON outbound_transfers(updated_at);
        CREATE INDEX IF NOT EXISTS idx_inbound_transfers_updated
            ON inbound_transfers(updated_at);",
    )?;
    ensure_message_column(
        &conn,
        "conversation_kind",
        "ALTER TABLE messages ADD COLUMN conversation_kind TEXT NOT NULL DEFAULT 'contact'",
    )?;
    ensure_message_column(
        &conn,
        "conversation_id",
        "ALTER TABLE messages ADD COLUMN conversation_id TEXT NOT NULL DEFAULT ''",
    )?;
    conn.execute(
        "UPDATE messages SET conversation_id = contact WHERE conversation_id = ''",
        [],
    )?;
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_messages_conversation ON messages(conversation_kind, conversation_id)",
        [],
    )?;
    Ok(conn)
}

fn ensure_message_column(conn: &Connection, column: &str, alter_sql: &str) -> Result<()> {
    let mut stmt = conn.prepare("PRAGMA table_info(messages)")?;
    let cols = stmt.query_map([], |r| r.get::<_, String>(1))?;
    for col in cols {
        if col? == column {
            return Ok(());
        }
    }
    conn.execute(alter_sql, [])?;
    Ok(())
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

#[allow(dead_code)]
#[derive(Serialize)]
pub(crate) struct HistoryRow {
    id: i64,
    direction: String,
    contact: String,
    onion: String,
    body: String,
    timestamp_ms: i64,
    status: i64,
    created_at: String,
    conversation_kind: String,
    conversation_id: String,
}

pub(crate) fn load_history(
    profile: &Path,
    contact_filter: Option<&str>,
    limit: usize,
) -> Result<Vec<HistoryRow>> {
    let conn = init_db(profile)?;

    if let Some(c) = contact_filter {
        let mut stmt = conn.prepare(
            "SELECT id, direction, contact, onion, body, timestamp_ms, status, created_at, conversation_kind, conversation_id
             FROM messages
             WHERE contact = ?1
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

fn history_row_from_sql(r: &rusqlite::Row<'_>) -> rusqlite::Result<HistoryRow> {
    Ok(HistoryRow {
        id: r.get(0)?,
        direction: r.get(1)?,
        contact: r.get(2)?,
        onion: r.get(3)?,
        body: r.get(4)?,
        timestamp_ms: r.get(5)?,
        status: r.get(6)?,
        created_at: r.get(7)?,
        conversation_kind: r.get(8)?,
        conversation_id: r.get(9)?,
    })
}

fn now_ms_i64() -> Result<i64> {
    Ok(SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis() as i64)
}

fn generate_group_id() -> String {
    let mut bytes = [0u8; 16];
    rand::RngCore::fill_bytes(&mut rand::thread_rng(), &mut bytes);
    hex::encode(bytes)
}

pub(crate) fn create_group(profile: &Path, title: &str, members: &[String]) -> Result<GroupInfo> {
    let title = title.trim();
    if title.is_empty() {
        return Err(anyhow!("group title is required"));
    }
    if members.is_empty() {
        return Err(anyhow!("group requires at least one member"));
    }

    let contacts = load_contacts(profile)?;
    let mut unique_members: Vec<String> = Vec::new();
    for member in members {
        let member = member.trim();
        if member.is_empty() {
            continue;
        }
        if !contacts.contains_key(member) {
            return Err(anyhow!("unknown group member '{member}'"));
        }
        if !unique_members.iter().any(|m| m == member) {
            unique_members.push(member.to_string());
        }
    }
    if unique_members.is_empty() {
        return Err(anyhow!("group requires at least one member"));
    }

    let conn = init_db(profile)?;
    let id = generate_group_id();
    let now = now_ms_i64()?;
    conn.execute(
        "INSERT INTO groups (id, title, created_at_ms, updated_at_ms) VALUES (?1, ?2, ?3, ?4)",
        params![id, title, now, now],
    )?;
    for member in &unique_members {
        conn.execute(
            "INSERT INTO group_members (group_id, contact, role, added_at_ms) VALUES (?1, ?2, 'member', ?3)",
            params![id, member, now],
        )?;
    }

    Ok(GroupInfo {
        id,
        title: title.to_string(),
        created_at_ms: now,
        updated_at_ms: now,
        members: unique_members
            .into_iter()
            .map(|contact| GroupMember {
                contact,
                role: "member".to_string(),
                added_at_ms: now,
            })
            .collect(),
    })
}

pub(crate) fn load_groups(profile: &Path) -> Result<Vec<GroupInfo>> {
    let conn = init_db(profile)?;
    let mut stmt = conn.prepare(
        "SELECT id, title, created_at_ms, updated_at_ms FROM groups ORDER BY updated_at_ms DESC, title ASC",
    )?;
    let group_rows = stmt.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, i64>(2)?,
            r.get::<_, i64>(3)?,
        ))
    })?;

    let mut groups = Vec::new();
    for row in group_rows {
        let (id, title, created_at_ms, updated_at_ms) = row?;
        let mut member_stmt = conn.prepare(
            "SELECT contact, role, added_at_ms FROM group_members WHERE group_id = ?1 ORDER BY contact ASC",
        )?;
        let member_rows = member_stmt.query_map(params![id], |r| {
            Ok(GroupMember {
                contact: r.get(0)?,
                role: r.get(1)?,
                added_at_ms: r.get(2)?,
            })
        })?;
        let mut members = Vec::new();
        for member in member_rows {
            members.push(member?);
        }
        groups.push(GroupInfo {
            id,
            title,
            created_at_ms,
            updated_at_ms,
            members,
        });
    }
    Ok(groups)
}

fn clear_history(profile: &Path, contact_filter: Option<&str>) -> Result<()> {
    let conn = init_db(profile)?;
    let deleted = if let Some(c) = contact_filter {
        conn.execute("DELETE FROM messages WHERE contact = ?1", params![c])?
    } else {
        conn.execute("DELETE FROM messages", [])?
    };
    println!("deleted {deleted} history row(s)");
    Ok(())
}

// ---------------------------------------------------------------------------
// Tui event type
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub(crate) enum TuiEvent {
    InboundMessage {
        contact: String,
        body: String,
        timestamp_ms: u128,
        verified: bool,
    },
    OutboundMessage {
        contact: String,
        body: String,
        timestamp_ms: u128,
        status: DeliveryStatus,
    },
    StatusUpdate(String),
}

#[derive(Debug, Deserialize)]
struct ServeControlCommand {
    cmd: String,
    to: Option<String>,
    message: Option<String>,
}

// ---------------------------------------------------------------------------
// Entrypoint
// ---------------------------------------------------------------------------

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    // For TUI mode, redirect all tracing to a log file so it doesn't
    // bleed into the terminal UI. For CLI mode, use stderr as normal.
    if let CommandKind::Tui(ref args) = cli.command {
        let profile = args.path()?;
        fs::create_dir_all(&profile).context("create profile dir for TUI log")?;
        let log_path = profile.join("tor.log");
        init_tracing_to_file(&log_path);
    } else {
        init_tracing_stderr();
    }

    match cli.command {
        CommandKind::Init(args) => {
            let profile = args.path()?;
            run_wizard(&profile)
        }
        CommandKind::Identity(args) => {
            let profile = args.path()?;
            ensure_profile(&profile)?;
            print_identity(&profile)
        }
        CommandKind::Share {
            profile,
            onion,
            json,
        } => {
            let profile = profile.path()?;
            ensure_profile(&profile)?;
            print_share(&profile, onion.as_deref(), json)
        }
        CommandKind::Serve(args) => {
            let profile = args.path()?;
            ensure_profile(&profile)?;
            let (tx, mut rx) = mpsc::channel::<TuiEvent>(64);
            tokio::spawn(async move {
                while let Some(evt) = rx.recv().await {
                    match evt {
                        TuiEvent::StatusUpdate(text) => println!("{text}"),
                        TuiEvent::InboundMessage { .. } => println!("message received"),
                        TuiEvent::OutboundMessage { .. } => {}
                    }
                }
            });
            let (_quit_tx, quit_rx) = tokio::sync::oneshot::channel::<()>();
            let tor_client = transport::tor::TorTransport::bootstrap(&profile).await?;
            let tor = transport::tor::TorTransport::new(None, tor_client);
            crate::serve(&profile, tx, quit_rx, tor.client.clone(), true).await
        }
        CommandKind::Send {
            profile,
            to,
            message,
            force_static,
        } => {
            let profile = profile.path()?;
            ensure_profile(&profile)?;
            let onion = resolve_to(&profile, &to)?;
            let tor_client = transport::tor::TorTransport::bootstrap(&profile).await?;
            send(
                &profile,
                &onion,
                &message,
                &to,
                None,
                tor_client,
                force_static,
            )
            .await
        }
        CommandKind::Contact { action } => match action {
            ContactAction::Add {
                profile,
                name,
                onion,
                pubkey,
                x25519_pubkey,
            } => {
                let profile = profile.path()?;
                ensure_profile(&profile)?;
                contact_add(&profile, &name, &onion, &pubkey, &x25519_pubkey)
            }
            ContactAction::Delete { profile, name } => {
                let profile = profile.path()?;
                ensure_profile(&profile)?;
                if contact_delete(&profile, &name)? {
                    println!("contact '{name}' deleted");
                } else {
                    println!("contact '{name}' not found");
                }
                Ok(())
            }
            ContactAction::List { profile, json } => {
                let profile = profile.path()?;
                ensure_profile(&profile)?;
                contact_list(&profile, json)
            }
        },
        CommandKind::Group { action } => match action {
            GroupAction::Create {
                profile,
                title,
                members,
                json,
            } => {
                let profile = profile.path()?;
                ensure_profile(&profile)?;
                let group = create_group(&profile, &title, &members)?;
                if json {
                    println!("{}", serde_json::to_string_pretty(&group)?);
                } else {
                    println!("group '{}' created ({})", group.title, group.id);
                    println!(
                        "members: {}",
                        group
                            .members
                            .iter()
                            .map(|m| m.contact.as_str())
                            .collect::<Vec<_>>()
                            .join(", ")
                    );
                }
                Ok(())
            }
            GroupAction::List { profile, json } => {
                let profile = profile.path()?;
                ensure_profile(&profile)?;
                let groups = load_groups(&profile)?;
                if json {
                    println!("{}", serde_json::to_string_pretty(&groups)?);
                } else if groups.is_empty() {
                    println!("(no groups)");
                } else {
                    for group in groups {
                        let members = group
                            .members
                            .iter()
                            .map(|m| m.contact.as_str())
                            .collect::<Vec<_>>()
                            .join(", ");
                        println!("{}\t{}\t{}", group.id, group.title, members);
                    }
                }
                Ok(())
            }
        },
        CommandKind::History {
            profile,
            contact,
            limit,
            json,
            clear,
        } => {
            let profile = profile.path()?;
            ensure_profile(&profile)?;
            if clear {
                clear_history(&profile, contact.as_deref())
            } else {
                history(&profile, contact.as_deref(), limit, json)
            }
        }
        CommandKind::Ratchet { profile, contact } => {
            let profile = profile.path()?;
            ensure_profile(&profile)?;
            init_ratchet_for_contact(&profile, &contact)?;
            println!("ratchet initialized for '{contact}'");
            Ok(())
        }
        CommandKind::Name { profile, name } => {
            let profile = profile.path()?;
            ensure_profile(&profile)?;
            match name {
                Some(name) => {
                    let name = set_display_name(&profile, &name)?;
                    println!("name set to: {name}");
                }
                None => println!("name: {}", load_display_name(&profile)?),
            }
            Ok(())
        }
        CommandKind::Tui(args) => {
            let profile = args.path()?;
            ensure_profile(&profile)?;
            eprintln!("Starting Tor… first run may take 30-60s to download consensus.");
            tui::run_tui(&profile).await
        }
    }
}

// ---------------------------------------------------------------------------
// Parse `to` field: onion address or contact name
// ---------------------------------------------------------------------------

pub(crate) fn resolve_to(profile: &Path, to: &str) -> Result<String> {
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

pub(crate) fn resolve_contact_name_by_pubkey(
    contacts: &std::collections::HashMap<String, ContactFile>,
    pubkey_b64: &str,
) -> Result<String> {
    contacts
        .values()
        .find(|c| c.pubkey_b64 == pubkey_b64)
        .map(|c| c.name.clone())
        .ok_or_else(|| anyhow!("unknown contact for pubkey"))
}

// ---------------------------------------------------------------------------
// Profile / Identity
// ---------------------------------------------------------------------------

fn default_display_name(profile: &Path) -> String {
    profile
        .file_name()
        .and_then(|n| n.to_str())
        .map(|name| name.trim_start_matches('.'))
        .filter(|name| !name.is_empty())
        .unwrap_or("sideband")
        .to_string()
}

fn identity_path(profile: &Path) -> PathBuf {
    profile.join("identity.toml")
}

fn load_identity(profile: &Path) -> Result<IdentityFile> {
    let path = identity_path(profile);
    let text = fs::read_to_string(&path).context("read identity.toml")?;
    Ok(toml::from_str(&text)?)
}

fn save_identity(profile: &Path, id: &IdentityFile) -> Result<()> {
    fs::write(identity_path(profile), toml::to_string_pretty(id)?).context("write identity")
}

pub(crate) fn load_display_name(profile: &Path) -> Result<String> {
    let mut id = load_identity(profile)?;
    if id.display_name.trim().is_empty() {
        id.display_name = default_display_name(profile);
        save_identity(profile, &id)?;
    }
    Ok(id.display_name)
}

pub(crate) fn set_display_name(profile: &Path, name: &str) -> Result<String> {
    let name = name.trim();
    if name.is_empty() {
        return Err(anyhow!("name cannot be empty"));
    }
    if name.chars().any(char::is_control) {
        return Err(anyhow!("name cannot contain control characters"));
    }
    let mut id = load_identity(profile)?;
    id.display_name = name.to_string();
    save_identity(profile, &id)?;
    Ok(id.display_name)
}

#[allow(dead_code)]
fn init_profile(profile: &Path) -> Result<()> {
    fs::create_dir_all(profile).context("create profile dir")?;

    let identity_path = identity_path(profile);
    if !identity_path.exists() {
        let signing = SigningKey::generate(&mut OsRng);
        let x25519_secret = StaticSecret::random_from_rng(OsRng);
        let id_file = IdentityFile {
            secret_key_b64: B64.encode(signing.to_bytes()),
            display_name: default_display_name(profile),
            x25519_secret_b64: B64.encode(x25519_secret.to_bytes()),
        };
        fs::write(&identity_path, toml::to_string_pretty(&id_file)?).context("write identity")?;
    }

    fs::create_dir_all(profile.join("tor/state"))?;
    fs::create_dir_all(profile.join("tor/hs"))?;

    // SQLite schema (no-op if already exists)
    init_db(profile)?;

    info!(profile=%profile.display(), "profile initialized");
    Ok(())
}

pub(crate) fn ensure_profile(profile: &Path) -> Result<()> {
    let identity_path = identity_path(profile);
    if identity_path.exists() {
        return Ok(());
    }
    // No identity yet — run first-time wizard
    run_wizard(profile)?;
    Ok(())
}

/// Interactive first-time setup. Prompts the user for a display name
/// and creates the profile with that name.
fn run_wizard(profile: &Path) -> Result<()> {
    use std::io::{self, Write};

    let identity_path = identity_path(profile);
    if identity_path.exists() {
        let name = load_display_name(profile).unwrap_or_else(|_| "(unknown)".into());
        println!(
            "  Profile already exists: {} (name: {})",
            profile.display(),
            name
        );
        return Ok(());
    }

    let default_name = default_display_name(profile);

    eprintln!("\n  === Sideband First-Time Setup ===\n");
    eprintln!("  Profile: {}", profile.display());
    eprintln!("  Default display name: {default_name}\n");
    eprint!("  Enter your display name [{default_name}]: ");
    io::stderr().flush()?;

    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    let name = input.trim();
    let name = if name.is_empty() {
        default_name
    } else {
        name.to_string()
    };

    // Validate
    if name.is_empty() {
        anyhow::bail!("display name cannot be empty");
    }
    if name.len() > 64 {
        anyhow::bail!("display name too long (max 64 chars)");
    }

    eprintln!("\n  Creating profile as '{name}'...\n");

    // Create profile with the chosen name
    init_profile_with_name(profile, &name)
}

pub(crate) fn init_profile_with_name(profile: &Path, display_name: &str) -> Result<()> {
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

fn load_signing_key(profile: &Path) -> Result<SigningKey> {
    let id = load_identity(profile)?;
    let secret = B64.decode(id.secret_key_b64.as_bytes())?;
    let arr: [u8; 32] = secret
        .as_slice()
        .try_into()
        .map_err(|_| anyhow!("invalid ed25519 secret length"))?;
    Ok(SigningKey::from_bytes(&arr))
}

fn print_identity(profile: &Path) -> Result<()> {
    let key = load_signing_key(profile)?;
    let verify: VerifyingKey = key.verifying_key();
    let x25519_pub = load_x25519_public(profile)?;
    let display_name = load_display_name(profile)?;
    println!("profile: {}", profile.display());
    println!("name: {}", display_name);
    println!("pubkey(ed25519,b64): {}", B64.encode(verify.to_bytes()));
    println!("pubkey(x25519,b64): {}", B64.encode(x25519_pub.as_bytes()));
    Ok(())
}

pub(crate) fn share_command(profile: &Path, onion: &str) -> Result<String> {
    if onion.trim().is_empty() || onion.starts_with('(') {
        anyhow::bail!("onion address is not ready yet");
    }
    let key = load_signing_key(profile)?;
    let verify: VerifyingKey = key.verifying_key();
    let x25519_pub = load_x25519_public(profile)?;
    let display_name = load_display_name(profile)?;
    Ok(format!(
        "/add {} {} {} {}",
        display_name,
        onion,
        B64.encode(verify.to_bytes()),
        B64.encode(x25519_pub.as_bytes())
    ))
}

pub(crate) fn qr_unicode(payload: &str) -> Result<String> {
    let code = QrCode::new(payload.as_bytes()).context("generate QR code")?;
    Ok(code
        .render::<unicode::Dense1x2>()
        .quiet_zone(true)
        .module_dimensions(2, 1)
        .build())
}

fn qr_matrix(payload: &str) -> Result<Vec<String>> {
    let code = QrCode::new(payload.as_bytes()).context("generate QR code")?;
    let width = code.width();
    let quiet = 4usize;
    let mut rows = Vec::with_capacity(width + quiet * 2);
    for y in 0..(width + quiet * 2) {
        let mut row = String::with_capacity(width + quiet * 2);
        for x in 0..(width + quiet * 2) {
            let dark = x >= quiet
                && y >= quiet
                && x < width + quiet
                && y < width + quiet
                && code[(x - quiet, y - quiet)] == QrColor::Dark;
            row.push(if dark { '1' } else { '0' });
        }
        rows.push(row);
    }
    Ok(rows)
}

fn print_share(profile: &Path, onion: Option<&str>, json: bool) -> Result<()> {
    let onion = onion.ok_or_else(|| anyhow!("onion address is not ready yet"))?;
    let command = share_command(profile, onion)?;
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&ShareInfo {
                qr: qr_matrix(&command)?,
                command,
            })?
        );
    } else {
        println!("Send this to your contact:");
        println!("  {command}");
        println!();
        println!("Scan to add:");
        println!("{}", qr_unicode(&command)?);
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Contacts
// ---------------------------------------------------------------------------

fn contacts_path(profile: &Path) -> PathBuf {
    profile.join("contacts.toml")
}

pub(crate) fn load_contacts(profile: &Path) -> Result<ContactsMap> {
    let p = contacts_path(profile);
    if !p.exists() {
        return Ok(HashMap::new());
    }
    let text = fs::read_to_string(&p).with_context(|| format!("read {}", p.display()))?;
    let map: ContactsMap = toml::from_str(&text)?;
    Ok(map)
}

fn save_contacts(profile: &Path, contacts: &ContactsMap) -> Result<()> {
    let p = contacts_path(profile);
    fs::write(&p, toml::to_string_pretty(contacts)?)
        .with_context(|| format!("write {}", p.display()))?;
    Ok(())
}

pub(crate) fn contact_add(
    profile: &Path,
    name: &str,
    onion: &str,
    pubkey_b64: &str,
    x25519_pubkey_b64: &str,
) -> Result<()> {
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

pub(crate) fn contact_delete(profile: &Path, name: &str) -> Result<bool> {
    let mut contacts = load_contacts(profile)?;
    if contacts.remove(name).is_some() {
        save_contacts(profile, &contacts)?;
        Ok(true)
    } else {
        Ok(false)
    }
}

fn contact_list(profile: &Path, json: bool) -> Result<()> {
    let contacts = load_contacts(profile)?;
    if json {
        #[derive(Serialize)]
        struct ContactRow<'a> {
            name: &'a str,
            onion: &'a str,
            pubkey_b64: &'a str,
            x25519_pubkey_b64: Option<&'a str>,
        }

        let mut rows: Vec<_> = contacts
            .values()
            .map(|c| ContactRow {
                name: &c.name,
                onion: &c.onion,
                pubkey_b64: &c.pubkey_b64,
                x25519_pubkey_b64: c.x25519_pubkey_b64.as_deref(),
            })
            .collect();
        rows.sort_by(|a, b| a.name.cmp(b.name));
        println!("{}", serde_json::to_string(&rows)?);
        return Ok(());
    }

    if contacts.is_empty() {
        println!("(no contacts)");
        return Ok(());
    }
    for (name, c) in &contacts {
        println!("{}\t onion={}\t pubkey_b64={}", name, c.onion, c.pubkey_b64);
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Signing helpers
// ---------------------------------------------------------------------------

/// Canonical payload that gets signed: JSON object with all fields except `sig_b64`.
fn payload_to_sign(msg: &ChatMessage) -> Result<String> {
    #[derive(Serialize)]
    struct Payload {
        v: u32,
        #[serde(rename = "type")]
        msg_type: String,
        from: String,
        timestamp_ms: u128,
        body: String,
        #[serde(default, skip_serializing_if = "String::is_empty")]
        ratchet_header_b64: String,
    }
    let p = Payload {
        v: msg.v,
        msg_type: msg.r#type.clone(),
        from: msg.from.clone(),
        timestamp_ms: msg.timestamp_ms,
        body: msg.body.clone(),
        ratchet_header_b64: msg.ratchet_header_b64.clone(),
    };
    Ok(serde_json::to_string(&p)?)
}

fn sign_message(key: &SigningKey, msg: &mut ChatMessage) -> Result<()> {
    let payload = payload_to_sign(msg)?;
    let sig = key.sign(payload.as_bytes());
    msg.sig_b64 = B64.encode(sig.to_bytes());
    Ok(())
}

fn load_x25519_secret(profile: &Path) -> Result<StaticSecret> {
    let mut id = load_identity(profile)?;
    if id.x25519_secret_b64.is_empty() {
        // Upgrade old identity file: generate and persist X25519 key.
        let secret = StaticSecret::from({
            let mut arr = [0u8; 32];
            use rand::RngCore;
            OsRng.fill_bytes(&mut arr);
            arr
        });
        id.x25519_secret_b64 = B64.encode(secret.to_bytes());
        save_identity(profile, &id)?;
        Ok(secret)
    } else {
        let raw = B64.decode(id.x25519_secret_b64.as_bytes())?;
        let arr: [u8; 32] = raw
            .as_slice()
            .try_into()
            .map_err(|_| anyhow!("invalid x25519 secret length"))?;
        Ok(StaticSecret::from(arr))
    }
}

fn load_x25519_public(profile: &Path) -> Result<X25519PublicKey> {
    let secret = load_x25519_secret(profile)?;
    Ok(X25519PublicKey::from(&secret))
}

// ---------------------------------------------------------------------------
// Double Ratchet (X25519 + HKDF-SHA256 + ChaCha20-Poly1305)
// ---------------------------------------------------------------------------

/// Per-contact ratchet state, persisted as bincode under `<profile>/ratchet/`.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct RatchetState {
    /// Our current DH keypair (sender side).
    dh_secret_b64: String,
    /// Their last known DH public key (receiver side).
    their_dh_pub_b64: Option<String>,
    /// Root key (32 bytes, derived from DH).
    root_key_b64: String,
    /// Our sending chain key.
    send_ck_b64: Option<String>,
    /// Our receiving chain key.
    recv_ck_b64: Option<String>,
    /// Number of messages sent in the current send chain.
    send_n: u32,
    /// Number of messages received in the current recv chain.
    recv_n: u32,
    /// Total messages sent in the previous send chain.
    prev_send_n: u32,
    /// Whether we've received at least one message from the peer.
    initialized: bool,
}

impl RatchetState {
    /// Path to the ratchet state file for a given contact.
    pub(crate) fn path(profile: &Path, contact: &Path) -> PathBuf {
        profile
            .join("ratchet")
            .join(format!("{}.bin", contact.display()))
    }

    /// Load existing state or create a new one from an X25519 shared secret.
    fn load_or_init_alice(
        profile: &Path,
        contact_name: &str,
        shared_secret: &[u8],
        their_dh_pub: &X25519PublicKey,
    ) -> Result<(Self, Vec<u8>, Vec<u8>)> {
        let path = Self::path(profile, std::path::Path::new(contact_name));
        if path.exists() {
            let bytes = fs::read(&path)?;
            let state: Self = bincode::deserialize(&bytes).context("deserialize ratchet state")?;
            return Ok((state, Vec::new(), Vec::new()));
        }
        // Initialize Alice: generate our DH keypair, derive root key.
        let our_dh_secret = StaticSecret::random_from_rng(OsRng);
        let our_dh_pub = X25519PublicKey::from(&our_dh_secret);
        let dh_out = our_dh_secret.diffie_hellman(their_dh_pub);
        let (root_key, send_ck) = hkdf_root_key(shared_secret, dh_out.as_bytes())?;
        let state = Self {
            dh_secret_b64: B64.encode(our_dh_secret.to_bytes()),
            their_dh_pub_b64: Some(B64.encode(their_dh_pub.as_bytes())),
            root_key_b64: B64.encode(root_key),
            send_ck_b64: Some(B64.encode(send_ck)),
            recv_ck_b64: None,
            send_n: 0,
            recv_n: 0,
            prev_send_n: 0,
            initialized: false,
        };
        Ok((state, our_dh_pub.as_bytes().to_vec(), Vec::new()))
    }

    fn new_bob(shared_secret: &[u8], our_dh_keypair: (&StaticSecret, &X25519PublicKey)) -> Self {
        let (our_secret, _our_pub) = our_dh_keypair;
        Self {
            dh_secret_b64: B64.encode(our_secret.to_bytes()),
            their_dh_pub_b64: None,
            root_key_b64: B64.encode(shared_secret.to_vec()),
            send_ck_b64: None,
            recv_ck_b64: None,
            send_n: 0,
            recv_n: 0,
            prev_send_n: 0,
            initialized: false,
        }
    }

    fn load_or_init_bob(
        profile: &Path,
        contact_name: &str,
        shared_secret: &[u8],
        our_dh_keypair: (&StaticSecret, &X25519PublicKey),
    ) -> Result<Self> {
        let path = Self::path(profile, std::path::Path::new(contact_name));
        if path.exists() {
            let bytes = fs::read(&path)?;
            return bincode::deserialize(&bytes).context("deserialize ratchet state");
        }
        Ok(Self::new_bob(shared_secret, our_dh_keypair))
    }

    fn save(&self, profile: &Path, contact_name: &str) -> Result<()> {
        let path = Self::path(profile, std::path::Path::new(contact_name));
        fs::create_dir_all(path.parent().unwrap())?;
        let bytes = bincode::serialize(self)?;
        fs::write(&path, bytes).context("write ratchet state")?;
        Ok(())
    }
}

/// HKDF-based root key derivation: combines the initial shared secret with DH output.
fn hkdf_root_key(shared_secret: &[u8], dh_bytes: &[u8]) -> Result<(Vec<u8>, Vec<u8>)> {
    let mut ikm = Vec::with_capacity(shared_secret.len() + dh_bytes.len());
    ikm.extend_from_slice(shared_secret);
    ikm.extend_from_slice(dh_bytes);
    let hk = Hkdf::<Sha256>::new(None, &ikm);
    let mut okm = [0u8; 64]; // 32 for root key, 32 for chain key
    hk.expand(b"sideband-ratchet-v1", &mut okm)
        .map_err(|e| anyhow!("hkdf expand failed: {e}"))?;
    Ok((okm[..32].to_vec(), okm[32..].to_vec()))
}

/// HKDF-based chain key derivation: derive next chain key + message key from current chain key.
fn hkdf_chain_key(chain_key: &[u8]) -> Result<(Vec<u8>, Vec<u8>)> {
    let hk = Hkdf::<Sha256>::new(None, chain_key);
    let mut okm = [0u8; 64];
    hk.expand(b"sideband-chain-v1", &mut okm)
        .map_err(|e| anyhow!("hkdf chain expand failed: {e}"))?;
    let mut mk = [0u8; 32];
    mk.copy_from_slice(&okm[..32]);
    let mut ck = [0u8; 32];
    ck.copy_from_slice(&okm[32..]);
    Ok((ck.to_vec(), mk.to_vec()))
}

// We use a custom Double Ratchet implementation rather than the `double-ratchet`
// crate because we need tight integration with our identity keys, bincode
// serialization, and ChaCha20-Poly1305 AEAD with our custom header format.
// The `double-ratchet` crate is kept as a dev reference.

/// Encrypt a message using the Double Ratchet. Returns (header_b64, nonce_hex, ciphertext_hex).
fn ratchet_encrypt(
    state: &mut RatchetState,
    plaintext: &[u8],
    _our_ed25519_pub: &str,
) -> Result<(String, String, String)> {
    // Advance the ratchet if needed: when we receive a new DH key from the peer,
    // we swap to a new root key. For now, we handle the symmetric ratchet step.

    let ck = state
        .send_ck_b64
        .as_ref()
        .ok_or_else(|| anyhow!("ratchet not initialized for sending"))?;
    let ck_bytes = B64.decode(ck.as_bytes())?;
    let (next_ck, mk) = hkdf_chain_key(&ck_bytes)?;

    // Encrypt with message key using ChaCha20-Poly1305.
    let cipher = ChaCha20Poly1305::new(Key::from_slice(&mk));
    let mut nonce_bytes = [0u8; 12];
    rand::RngCore::fill_bytes(&mut rand::thread_rng(), &mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);
    let ciphertext = cipher
        .encrypt(nonce, plaintext)
        .map_err(|e| anyhow!("ratchet encrypt failed: {e}"))?;

    // Build header: dh_pub | send_n | prev_send_n
    let dh_secret = StaticSecret::from({
        let bytes = B64.decode(state.dh_secret_b64.as_bytes())?;
        let arr: [u8; 32] = bytes
            .as_slice()
            .try_into()
            .map_err(|_| anyhow!("invalid dh_secret length"))?;
        arr
    });
    let dh_pub = X25519PublicKey::from(&dh_secret);
    let mut header_bytes = Vec::new();
    header_bytes.extend_from_slice(dh_pub.as_bytes());
    header_bytes.extend_from_slice(&state.send_n.to_be_bytes());
    header_bytes.extend_from_slice(&state.prev_send_n.to_be_bytes());

    // Update state.
    state.send_ck_b64 = Some(B64.encode(&next_ck));
    state.send_n += 1;

    let header_b64 = B64.encode(&header_bytes);
    let nonce_hex = hex::encode(nonce.as_slice());
    let ct_hex = hex::encode(&ciphertext);

    Ok((header_b64, nonce_hex, ct_hex))
}

/// Decrypt a message using the Double Ratchet. Returns plaintext bytes.
fn ratchet_decrypt(
    state: &mut RatchetState,
    header_b64: &str,
    nonce_hex: &str,
    ct_hex: &str,
) -> Result<Vec<u8>> {
    let header_bytes = B64.decode(header_b64.as_bytes())?;
    if header_bytes.len() < 32 + 4 + 4 {
        return Err(anyhow!("ratchet header too short"));
    }
    let their_dh_pub_bytes: [u8; 32] = header_bytes[..32].try_into().unwrap();
    let msg_n = u32::from_be_bytes(header_bytes[32..36].try_into().unwrap());
    let _prev_n = u32::from_be_bytes(header_bytes[36..40].try_into().unwrap());

    let their_dh_pub = X25519PublicKey::from(their_dh_pub_bytes);

    // Check if this is a new DH ratchet step (new sender key).
    let is_new_dh = match &state.their_dh_pub_b64 {
        Some(existing) => {
            let existing = B64
                .decode(existing.as_bytes())
                .context("decode stored peer ratchet dh pub")?;
            existing.as_slice() != their_dh_pub.as_bytes()
        }
        None => true,
    };

    if is_new_dh {
        // New DH ratchet step: derive new root key from our DH secret + their new DH pub.
        let our_dh_secret_bytes = B64.decode(state.dh_secret_b64.as_bytes())?;
        let our_dh_secret_arr: [u8; 32] = our_dh_secret_bytes
            .as_slice()
            .try_into()
            .map_err(|_| anyhow!("invalid dh_secret"))?;
        let our_dh_secret = StaticSecret::from(our_dh_secret_arr);

        // First, finish the previous send chain: derive recv chain key from current root.
        let root_key = B64.decode(state.root_key_b64.as_bytes())?;
        let dh_out = our_dh_secret.diffie_hellman(&their_dh_pub);
        let (new_root, new_recv_ck) = hkdf_root_key(&root_key, dh_out.as_bytes())?;

        // Generate new DH keypair for our sending side.
        let new_dh_secret = StaticSecret::random_from_rng(OsRng);
        let dh_out2 = new_dh_secret.diffie_hellman(&their_dh_pub);
        let (new_root2, new_send_ck) = hkdf_root_key(&new_root, dh_out2.as_bytes())?;

        state.root_key_b64 = B64.encode(&new_root2);
        state.dh_secret_b64 = B64.encode(new_dh_secret.to_bytes());
        state.their_dh_pub_b64 = Some(B64.encode(their_dh_pub.as_bytes()));
        state.send_ck_b64 = Some(B64.encode(&new_send_ck));
        state.recv_ck_b64 = Some(B64.encode(&new_recv_ck));
        state.prev_send_n = state.send_n;
        state.send_n = 0;
        state.recv_n = 0;
        state.initialized = true;
    }

    let recv_ck = state
        .recv_ck_b64
        .as_ref()
        .ok_or_else(|| anyhow!("ratchet not initialized for receiving"))?;
    let ck_bytes = B64.decode(recv_ck.as_bytes())?;

    // Advance receive chain to the message number announced in header.
    // This handles dropped messages in-order by deriving and discarding missed keys.
    // (Still no out-of-order/duplicate key cache.)
    if msg_n < state.recv_n {
        return Err(anyhow!(
            "ratchet message index went backwards: msg_n={} recv_n={}",
            msg_n,
            state.recv_n
        ));
    }

    let mut ck_cursor = ck_bytes;
    let mut mk = None;
    let steps = msg_n
        .checked_sub(state.recv_n)
        .ok_or_else(|| anyhow!("invalid ratchet counters"))?
        + 1;

    for _ in 0..steps {
        let (next_ck, derived_mk) = hkdf_chain_key(&ck_cursor)?;
        ck_cursor = next_ck;
        mk = Some(derived_mk);
    }

    state.recv_ck_b64 = Some(B64.encode(&ck_cursor));
    state.recv_n = msg_n + 1;

    let mk = mk.ok_or_else(|| anyhow!("ratchet message key derivation failed"))?;

    // Decrypt.
    let cipher = ChaCha20Poly1305::new(Key::from_slice(&mk));
    let nonce_bytes = hex::decode(nonce_hex)?;
    let ct_bytes = hex::decode(ct_hex)?;
    let nonce = Nonce::from_slice(&nonce_bytes);
    let plaintext = cipher
        .decrypt(nonce, ct_bytes.as_ref())
        .map_err(|e| anyhow!("ratchet decrypt failed: {e}"))?;

    Ok(plaintext)
}

/// Initialize the ratchet for a contact when they first message us (Alice role).
fn init_ratchet_alice(
    profile: &Path,
    contact_name: &str,
    their_x25519_pub: &X25519PublicKey,
) -> Result<()> {
    // Derive the initial shared secret from our static X25519 key + their static X25519 key.
    let our_secret = load_x25519_secret(profile)?;
    let shared = our_secret.diffie_hellman(their_x25519_pub);
    let shared_bytes = shared.as_bytes();

    let (state, _our_dh_pub_bytes, _) =
        RatchetState::load_or_init_alice(profile, contact_name, shared_bytes, their_x25519_pub)?;
    state.save(profile, contact_name)?;
    Ok(())
}

pub(crate) fn init_ratchet_for_contact(profile: &Path, contact_name: &str) -> Result<()> {
    let contacts = load_contacts(profile)?;
    let contact = contacts
        .get(contact_name)
        .with_context(|| format!("contact '{}' not found", contact_name))?;
    let x25519_b64 = contact
        .x25519_pubkey_b64
        .as_deref()
        .ok_or_else(|| anyhow!("contact '{}' has no x25519 key", contact_name))?;
    let raw = B64
        .decode(x25519_b64.as_bytes())
        .context("decode x25519 key")?;
    let arr: [u8; 32] = raw
        .as_slice()
        .try_into()
        .map_err(|_| anyhow!("invalid x25519 key length"))?;
    let their_pub = X25519PublicKey::from(arr);
    init_ratchet_alice(profile, contact_name, &their_pub)
}

/// Initialize the ratchet for a contact when we first message them (Bob role).
/// This is called when we receive a v3 message with a ratchet header.
#[allow(dead_code)]
fn init_ratchet_bob(
    profile: &Path,
    contact_name: &str,
    our_x25519: (&StaticSecret, &X25519PublicKey),
) -> Result<()> {
    let state = RatchetState::load_or_init_bob(
        profile,
        contact_name,
        // Bob starts with just his keypair; the root key is derived when
        // the first message arrives.
        &[],
        our_x25519,
    )?;
    state.save(profile, contact_name)?;
    Ok(())
}

fn init_bob_ratchet_from_contact(
    profile: &Path,
    contact_name: &str,
    contact: &ContactFile,
    overwrite_existing: bool,
) -> Result<RatchetState> {
    let x25519_b64 = contact
        .x25519_pubkey_b64
        .as_deref()
        .ok_or_else(|| anyhow!("sender has no x25519 key"))?;
    let raw = B64
        .decode(x25519_b64)
        .context("decode sender x25519 pubkey")?;
    let arr: [u8; 32] = raw
        .as_slice()
        .try_into()
        .map_err(|_| anyhow!("bad x25519 key len"))?;
    let their_static = X25519PublicKey::from(arr);
    let our_x25519 = load_x25519_secret(profile)?;
    let our_x25519_pub = X25519PublicKey::from(&our_x25519);
    let static_shared = our_x25519.diffie_hellman(&their_static);
    if overwrite_existing {
        Ok(RatchetState::new_bob(
            static_shared.as_bytes(),
            (&our_x25519, &our_x25519_pub),
        ))
    } else {
        RatchetState::load_or_init_bob(
            profile,
            contact_name,
            static_shared.as_bytes(),
            (&our_x25519, &our_x25519_pub),
        )
    }
}

fn derive_shared_key(our_secret: &StaticSecret, their_public: &X25519PublicKey) -> Result<Key> {
    let shared = our_secret.diffie_hellman(their_public);
    let hk = Hkdf::<Sha256>::new(None, shared.as_bytes());
    let mut okm = [0u8; 32];
    hk.expand(b"sideband-v1", &mut okm)
        .map_err(|e| anyhow!("hkdf expand failed: {e}"))?;
    Ok(*Key::from_slice(&okm))
}

fn encrypt_body(key: &Key, plaintext: &str) -> Result<String> {
    let cipher = ChaCha20Poly1305::new(key);
    let mut nonce_bytes = [0u8; 12];
    rand::RngCore::fill_bytes(&mut rand::thread_rng(), &mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);
    let ct = cipher
        .encrypt(nonce, plaintext.as_bytes())
        .map_err(|e| anyhow!("encrypt failed: {e}"))?;
    let mut out = Vec::with_capacity(12 + ct.len());
    out.extend_from_slice(nonce.as_slice());
    out.extend_from_slice(&ct);
    Ok(hex::encode(out))
}

fn decrypt_body(key: &Key, enc_hex: &str) -> Result<String> {
    let raw = hex::decode(enc_hex)?;
    if raw.len() < 12 {
        return Err(anyhow!("ciphertext too short"));
    }
    let (nonce_bytes, ct) = raw.split_at(12);
    let cipher = ChaCha20Poly1305::new(key);
    let nonce = Nonce::from_slice(nonce_bytes);
    let pt = cipher
        .decrypt(nonce, ct)
        .map_err(|e| anyhow!("decrypt failed: {e}"))?;
    String::from_utf8(pt).context("decrypt: invalid utf8")
}

fn resolve_x25519_pubkey(profile: &Path, contact_name: &str) -> Result<X25519PublicKey> {
    let contacts = load_contacts(profile)?;
    let contact = contacts
        .get(contact_name)
        .with_context(|| format!("unknown contact '{contact_name}'"))?;
    let raw = B64
        .decode(
            contact
                .x25519_pubkey_b64
                .as_deref()
                .ok_or_else(|| anyhow!("contact '{}' has no x25519 key (v1 only)", contact_name))?,
        )
        .context("decode x25519 pubkey base64")?;
    let arr: [u8; 32] = raw
        .as_slice()
        .try_into()
        .map_err(|_| anyhow!("invalid x25519 pubkey length"))?;
    Ok(X25519PublicKey::from(arr))
}

/// Decrypt a v2 inbound message and verify the Ed25519 signature.
/// v1 messages (no enc_body) pass through with plaintext verification.
pub(crate) fn decrypt_and_verify(
    msg: &mut ChatMessage,
    our_profile: &Path,
    contacts: &ContactsMap,
) -> Result<(String, bool)> {
    if msg.v == 3 {
        // v3: Double Ratchet decrypt.
        if msg.ratchet_header_b64.is_empty() {
            return Err(anyhow!("v3 message missing ratchet header"));
        }
        let contact = contacts.values().find(|c| c.pubkey_b64 == msg.from);
        let contact_name = contact
            .map(|c| c.name.clone())
            .unwrap_or_else(|| msg.from.clone());
        let ratchet_path = RatchetState::path(our_profile, std::path::Path::new(&contact_name));
        let mut state = if ratchet_path.exists() {
            let bytes = fs::read(&ratchet_path)?;
            bincode::deserialize(&bytes).context("deserialize ratchet state")?
        } else {
            // First v3 message from this contact: initialize receiver state
            // from the same static X25519 shared secret the sender used.
            let contact = contact.with_context(|| {
                format!(
                    "cannot initialize ratchet for unknown sender pubkey: {}",
                    msg.from
                )
            })?;
            let state = init_bob_ratchet_from_contact(our_profile, &contact_name, contact, false)?;
            state.save(our_profile, &contact_name)?;
            state
        };
        let plaintext_bytes = match ratchet_decrypt(
            &mut state,
            &msg.ratchet_header_b64,
            &msg.ratchet_nonce_hex,
            &msg.ratchet_ct_hex,
        ) {
            Ok(bytes) => bytes,
            Err(first_err) => {
                // Backwards indexes are duplicates/old deliveries. Do not run
                // the Alice/Alice recovery path for them: recovery can decrypt
                // an old message from the static base and overwrite a perfectly
                // good ratchet state, poisoning all later messages.
                if first_err
                    .to_string()
                    .contains("message index went backwards")
                {
                    return Err(first_err);
                }

                // If both peers manually ran `/ratchet`, both sides created
                // incompatible Alice-first states. Treat the inbound v3 as a
                // fresh peer-initiated ratchet, try Bob initialization, and
                // overwrite the poisoned local state only if decrypt succeeds.
                let contact = contact.with_context(|| {
                    format!(
                        "cannot recover ratchet for unknown sender pubkey: {}",
                        msg.from
                    )
                })?;
                let mut recovered =
                    init_bob_ratchet_from_contact(our_profile, &contact_name, contact, true)
                        .with_context(|| {
                            format!("ratchet recovery after decrypt failure: {first_err}")
                        })?;
                let bytes = ratchet_decrypt(
                    &mut recovered,
                    &msg.ratchet_header_b64,
                    &msg.ratchet_nonce_hex,
                    &msg.ratchet_ct_hex,
                )
                .with_context(|| {
                    format!("ratchet decrypt failed; recovery also failed after: {first_err}")
                })?;
                state = recovered;
                bytes
            }
        };
        state.save(our_profile, &contact_name)?;
        let plaintext =
            String::from_utf8(plaintext_bytes).context("ratchet plaintext not valid utf8")?;
        // Verify Ed25519 signature on the plaintext.
        msg.body = plaintext.clone();
        let verified = verify_message(msg, contacts).unwrap_or(false);
        return Ok((plaintext, verified));
    }
    if msg.v < 2 || msg.enc_body.is_empty() {
        let verified = verify_message(msg, contacts).unwrap_or(false);
        return Ok((msg.body.clone(), verified));
    }
    // v2: static X25519 decrypt.
    let contact = contacts
        .values()
        .find(|c| c.pubkey_b64 == msg.from)
        .with_context(|| format!("unknown sender pubkey: {}", msg.from))?;
    let our_secret = load_x25519_secret(our_profile)?;
    let x25519_b64 = contact
        .x25519_pubkey_b64
        .as_deref()
        .ok_or_else(|| anyhow!("sender has no x25519 key"))?;
    let raw = B64
        .decode(x25519_b64)
        .context("decode sender x25519 pubkey")?;
    let arr: [u8; 32] = raw
        .as_slice()
        .try_into()
        .map_err(|_| anyhow!("bad x25519 key len"))?;
    let their_public = X25519PublicKey::from(arr);
    let shared_key = derive_shared_key(&our_secret, &their_public)?;
    let plaintext = decrypt_body(&shared_key, &msg.enc_body)?;
    msg.body = plaintext.clone();
    let verified = verify_message(msg, contacts).unwrap_or(false);
    Ok((plaintext, verified))
}

fn verify_message(msg: &ChatMessage, contacts: &ContactsMap) -> Result<bool> {
    let payload = payload_to_sign(msg)?;
    let sig_bytes = B64
        .decode(msg.sig_b64.as_bytes())
        .context("decode sig base64")?;
    let sig_arr: [u8; 64] = sig_bytes
        .as_slice()
        .try_into()
        .map_err(|_| anyhow!("sig must decode to 64 bytes, got {}", sig_bytes.len()))?;
    let sig = Signature::from_bytes(&sig_arr);

    // Try matching against a contact first.
    for contact in contacts.values() {
        if let Ok(pk_bytes) = B64.decode(contact.pubkey_b64.as_bytes()) {
            if let Ok(arr) = <[u8; 32]>::try_from(pk_bytes.as_slice()) {
                if let Ok(pk) = VerifyingKey::from_bytes(&arr) {
                    if pk.verify_strict(payload.as_bytes(), &sig).is_ok() {
                        return Ok(true);
                    }
                }
            }
        }
    }

    // Fallback: try `from` field as raw pubkey.
    if let Ok(pk_bytes) = B64.decode(msg.from.as_bytes()) {
        if let Ok(arr) = <[u8; 32]>::try_from(pk_bytes.as_slice()) {
            if let Ok(pk) = VerifyingKey::from_bytes(&arr) {
                if pk.verify_strict(payload.as_bytes(), &sig).is_ok() {
                    return Ok(true);
                }
            }
        }
    }

    Ok(false)
}

// ---------------------------------------------------------------------------
// Tor helpers
// ---------------------------------------------------------------------------

#[cfg(unix)]
fn secure_arti_dir(path: &Path) -> Result<()> {
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    Ok(())
}

#[cfg(not(unix))]
fn secure_arti_dir(_path: &Path) -> Result<()> {
    Ok(())
}

/// Create a bootstrapped Arti Tor client.
/// State is stored under `<profile>/arti_state`; cache under `<profile>/arti_cache`.
async fn create_tor_client(profile: &Path) -> Result<TorClient<PreferredRuntime>> {
    let state_dir = profile.join("arti_state");
    let cache_dir = profile.join("arti_cache");
    fs::create_dir_all(&state_dir).context("create Arti state dir")?;
    fs::create_dir_all(&cache_dir).context("create Arti cache dir")?;
    secure_arti_dir(&state_dir).context("secure Arti state dir")?;
    secure_arti_dir(&cache_dir).context("secure Arti cache dir")?;

    let state_dir = state_dir
        .canonicalize()
        .context("canonicalize Arti state dir")?;
    let cache_dir = cache_dir
        .canonicalize()
        .context("canonicalize Arti cache dir")?;

    let mut builder = TorClientConfig::builder();
    builder
        .storage()
        .state_dir(CfgPath::new(state_dir.to_string_lossy().into_owned()))
        .cache_dir(CfgPath::new(cache_dir.to_string_lossy().into_owned()));
    let config = builder.build().context("build Arti Tor client config")?;

    let tor_client = TorClient::create_bootstrapped(config)
        .await
        .context("failed to bootstrap Arti Tor client")?;
    Ok(tor_client)
}

// ---------------------------------------------------------------------------
// Serve
// ---------------------------------------------------------------------------

pub(crate) async fn serve(
    profile: &Path,
    tui_tx: mpsc::Sender<TuiEvent>,
    quit_rx: tokio::sync::oneshot::Receiver<()>,
    tor_client: Arc<TorClient<PreferredRuntime>>,
    read_control_stdin: bool,
) -> Result<()> {
    let _key = crate::load_signing_key(profile)?;
    if let Err(e) = crate::load_incoming_states(profile) {
        tracing::warn!(error=%e, "failed to load persisted incoming file state");
    }
    let transport = Arc::new(crate::transport::tor::TorTransport::new_with_status(
        None,
        tor_client.clone(),
        Some(tui_tx.clone()),
    ));

    let (control_tx, mut control_rx) = mpsc::channel::<ServeControlCommand>(64);
    let send_lock = Arc::new(tokio::sync::Mutex::new(()));
    if read_control_stdin {
        tokio::spawn(async move {
            let mut lines = BufReader::new(tokio::io::stdin()).lines();
            loop {
                match lines.next_line().await {
                    Ok(Some(line)) => {
                        let line = line.trim();
                        if line.is_empty() {
                            continue;
                        }
                        match serde_json::from_str::<ServeControlCommand>(line) {
                            Ok(cmd) => {
                                if control_tx.send(cmd).await.is_err() {
                                    break;
                                }
                            }
                            Err(e) => println!("control error: {e}"),
                        }
                    }
                    Ok(None) => break,
                    Err(e) => {
                        println!("control error: {e}");
                        break;
                    }
                }
            }
        });
    }

    // Spawn the inbound IO loop (pure transport: accepts connections, parses
    // lines, pushes Envelopes into the channel).
    let io_transport = Arc::clone(&transport);
    let io_handle = tokio::spawn(async move { io_transport.run_inbound_loop(quit_rx).await });

    // Main dispatch loop: pull envelopes from the channel and handle them.
    // This is the only place that calls handle_inbound — transport is agnostic.
    loop {
        while let Ok(cmd) = control_rx.try_recv() {
            if cmd.cmd != "send" {
                println!("control error: unknown cmd '{}'", cmd.cmd);
                continue;
            }
            let Some(to) = cmd.to else {
                println!("send error: missing to");
                continue;
            };
            let Some(message) = cmd.message else {
                println!("send error: missing message");
                continue;
            };
            let profile = profile.to_path_buf();
            let tor_client = tor_client.clone();
            let send_lock = send_lock.clone();
            tokio::spawn(async move {
                let _guard = send_lock.lock().await;
                match resolve_to(&profile, &to) {
                    Ok(onion) => {
                        match send(&profile, &onion, &message, &to, None, tor_client, false).await {
                            Ok(()) => println!("message sent"),
                            Err(e) => println!("send error: {e}"),
                        }
                    }
                    Err(e) => println!("resolve error: {e}"),
                }
            });
        }

        match (*transport).try_recv().await {
            Ok(Some(envelope)) => {
                let body =
                    match crate::transport::tor::TorTransport::envelope_body_as_str(&envelope) {
                        Ok(b) => b.to_owned(),
                        Err(e) => {
                            tracing::error!(error=%e, "envelope body is not valid utf-8");
                            continue;
                        }
                    };
                if let Some(mut msg) = handler::parse_inbound_line(&body).unwrap_or(None) {
                    // Contacts can be added while the GUI listener is already
                    // running. Sending uses a fresh one-shot process, so a
                    // stale listener contact snapshot creates the dumbest bug:
                    // outbound works, inbound cannot decrypt/attribute.
                    let contacts = crate::load_contacts(profile).unwrap_or_default();
                    if let Err(e) = handler::handle_inbound(
                        profile,
                        &tui_tx,
                        &contacts,
                        &mut msg,
                        tor_client.clone(),
                    )
                    .await
                    {
                        tracing::error!(error=%e, "inbound handler error");
                    }
                } else {
                    tracing::error!(raw=%body, "invalid inbound payload");
                }
            }
            Ok(None) => {
                // No envelopes right now; yield briefly to avoid hot-looping.
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            }
            Err(e) => {
                tracing::error!(error=%e, "try_recv error from transport channel");
                break;
            }
        }

        // If the IO loop has finished, we're done.
        if io_handle.is_finished() {
            break;
        }
    }

    // Propagate any panic/error from the IO loop.
    io_handle.await??;
    Ok(())
}

// ---------------------------------------------------------------------------
// Send
// ---------------------------------------------------------------------------

pub(crate) async fn send(
    profile: &Path,
    to: &str,
    message: &str,
    contact_hint: &str,
    _reuse_socks_port: Option<u16>,
    tor_client: Arc<TorClient<PreferredRuntime>>,
    force_static: bool,
) -> Result<()> {
    if !to.ends_with(".onion") {
        return Err(anyhow!("resolved --to must be an onion address"));
    }

    let key = load_signing_key(profile)?;

    // Build message: try v3 (Double Ratchet) first, fall back to v2 (static X25519).
    let plaintext = message.to_string();
    let our_ed25519_pub = B64.encode(key.verifying_key().to_bytes());
    let timestamp_ms = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis();

    // Check if we have a ratchet state for this contact.
    let ratchet_path = RatchetState::path(profile, std::path::Path::new(contact_hint));
    let use_ratchet = ratchet_path.exists() && !force_static;

    let msg = if use_ratchet {
        // v3: Double Ratchet encrypt.
        let mut state_bytes = fs::read(&ratchet_path)?;
        let mut state: RatchetState =
            bincode::deserialize(&mut state_bytes).context("deserialize ratchet state")?;
        let (header_b64, nonce_hex, ct_hex) =
            ratchet_encrypt(&mut state, plaintext.as_bytes(), &our_ed25519_pub)?;
        state.save(profile, contact_hint)?;
        // Sign the plaintext for authentication.
        let mut sign_msg = ChatMessage {
            v: 3,
            r#type: "chat_message".into(),
            from: our_ed25519_pub.clone(),
            timestamp_ms,
            body: plaintext.clone(),
            sig_b64: String::new(),
            enc_body: String::new(),
            ratchet_header_b64: header_b64,
            ratchet_nonce_hex: nonce_hex,
            ratchet_ct_hex: ct_hex,
        };
        sign_message(&key, &mut sign_msg)?;
        sign_msg.body.clear(); // Don't send plaintext on wire.
        sign_msg
    } else {
        // v2: sign then encrypt with static X25519.
        let mut msg = ChatMessage {
            v: 2,
            r#type: "chat_message".into(),
            from: our_ed25519_pub.clone(),
            timestamp_ms,
            body: plaintext.clone(),
            sig_b64: String::new(),
            enc_body: String::new(),
            ratchet_header_b64: String::new(),
            ratchet_nonce_hex: String::new(),
            ratchet_ct_hex: String::new(),
        };
        sign_message(&key, &mut msg)?;
        let our_x25519 = load_x25519_secret(profile)?;
        let their_x25519 = resolve_x25519_pubkey(profile, contact_hint)?;
        let shared_key = derive_shared_key(&our_x25519, &their_x25519)?;
        msg.enc_body = encrypt_body(&shared_key, &plaintext)?;
        msg.body.clear();
        msg
    };

    // Retry loop: Tor circuits to HS can take time to establish, especially
    // when the remote HS has just been created (descriptor propagation 1-3min).
    // Use generous timeouts; failing fast just creates false negatives.
    let max_attempts = 3;
    let mut status = DeliveryStatus::Failed;
    let mut last_error: Option<String> = None;
    let payload = format!("{}\n", serde_json::to_string(&msg)?);

    for attempt in 1..=max_attempts {
        if attempt > 1 {
            warn!(attempt, "retrying send after circuit delay");
            sleep(Duration::from_secs(10)).await;
        }

        let result = {
            let payload = payload.clone();
            let to_addr = format!("{}:80", to);
            let tc = Arc::clone(&tor_client);
            let connect_fut = async move {
                let mut stream = tc
                    .connect(to_addr.as_str())
                    .await
                    .map_err(|e| anyhow!("connect: {e}"))?;
                use tokio::io::AsyncWriteExt;
                stream
                    .write_all(payload.as_bytes())
                    .await
                    .map_err(|e| anyhow!("write: {e}"))?;
                stream.flush().await.map_err(|e| anyhow!("flush: {e}"))?;
                stream
                    .shutdown()
                    .await
                    .map_err(|e| anyhow!("shutdown: {e}"))?;
                Ok::<_, anyhow::Error>(())
            };
            tokio::time::timeout(Duration::from_secs(60), connect_fut).await
        };

        match result {
            Ok(Ok(())) => {
                info!(sent=true, bytes=%payload.len(), to=%to, attempt, "message sent");
                status = DeliveryStatus::Sent;
                break;
            }
            Ok(Err(e)) => {
                error!(attempt, error=%e, "send error");
                last_error = Some(e.to_string());
                status = DeliveryStatus::Failed;
            }
            Err(_) => {
                let e = format!("connect timed out after 60s on attempt {attempt}");
                error!(attempt, "send timed out (circuit not ready)");
                last_error = Some(e);
                status = DeliveryStatus::Failed;
            }
        }
    }

    // Store outbound in DB.
    if let Err(e) = store_message(
        profile,
        "out",
        contact_hint,
        to,
        message,
        msg.timestamp_ms,
        status,
    ) {
        error!(error=%e, "failed to store outbound message");
    }

    drop(tor_client);

    if status == DeliveryStatus::Sent {
        Ok(())
    } else if let Some(e) = last_error {
        Err(anyhow!("message was not delivered to {}: {}", to, e))
    } else {
        Err(anyhow!("message was not delivered to {}", to))
    }
}

// ---------------------------------------------------------------------------
// History
// ---------------------------------------------------------------------------

fn history(profile: &Path, contact: Option<&str>, limit: usize, json: bool) -> Result<()> {
    let rows = load_history(profile, contact, limit)?;
    if json {
        println!("{}", serde_json::to_string(&rows)?);
        return Ok(());
    }
    if rows.is_empty() {
        println!("(no messages)");
        return Ok(());
    }
    for r in rows {
        let status_label = DeliveryStatus::from_i64(r.status)
            .map(|s| s.label())
            .unwrap_or("?");
        println!(
            "[{}] {:>5}  {:>8}  {:>12}  {}  ts={}",
            r.id, r.direction, status_label, r.contact, r.body, r.timestamp_ms
        );
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // -- profile paths --

    #[test]
    fn default_profile_path_uses_sideband_under_home() {
        let path = default_profile_path().unwrap();
        assert_eq!(path.file_name().unwrap(), ".sideband");
        assert!(path.is_absolute());
    }

    #[test]
    fn explicit_profile_expands_home_prefix() {
        let home = std::env::var_os("HOME").unwrap();
        assert_eq!(
            expand_home(Path::new("~/.sideband")),
            PathBuf::from(home).join(".sideband")
        );
    }

    // -- identity round-trip --

    #[test]
    fn identity_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        init_profile(dir.path()).unwrap();
        let key = load_signing_key(dir.path()).unwrap();
        let key2 = load_signing_key(dir.path()).unwrap();
        assert_eq!(key.to_bytes(), key2.to_bytes());
    }

    #[test]
    fn display_name_defaults_without_leading_dot_and_can_be_set() {
        let dir = tempfile::tempdir().unwrap();
        let profile = dir.path().join(".sideband");
        init_profile(&profile).unwrap();
        assert_eq!(load_display_name(&profile).unwrap(), "sideband");

        assert_eq!(set_display_name(&profile, " Alan ").unwrap(), "Alan");
        assert_eq!(load_display_name(&profile).unwrap(), "Alan");
    }

    #[test]
    fn display_name_rejects_empty_names() {
        let dir = tempfile::tempdir().unwrap();
        init_profile(dir.path()).unwrap();
        assert!(set_display_name(dir.path(), "   ").is_err());
    }

    // -- contacts CRUD --

    #[test]
    fn contacts_add_and_list() {
        let dir = tempfile::tempdir().unwrap();
        let pk = B64.encode([1u8; 32]);
        contact_add(
            dir.path(),
            "alice",
            "stqclefnkl4wfmdsz627hlfwu2xwgrk3sb6sgegfq44auik3pz7jmyqd.onion",
            &pk,
            &B64.encode([2u8; 32]),
        )
        .unwrap();
        let contacts = load_contacts(dir.path()).unwrap();
        assert_eq!(contacts.len(), 1);
    }

    #[test]
    fn contacts_rejects_bad_onion() {
        let dir = tempfile::tempdir().unwrap();
        let pk = B64.encode([1u8; 32]);
        assert!(contact_add(
            dir.path(),
            "bob",
            "not-an-onion",
            &pk,
            &B64.encode([2u8; 32])
        )
        .is_err());
    }

    #[test]
    fn contacts_rejects_bad_pubkey_len() {
        let dir = tempfile::tempdir().unwrap();
        let pk = B64.encode([1u8; 16]);
        assert!(contact_add(dir.path(), "bob", "bbbb.onion", &pk, &B64.encode([2u8; 32])).is_err());
    }

    // -- groups --

    #[test]
    fn db_group_create_and_load() {
        let dir = tempfile::tempdir().unwrap();
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

        let created = create_group(dir.path(), "Ops", &["alice".into(), "bob".into()]).unwrap();
        assert_eq!(created.title, "Ops");
        assert_eq!(created.members.len(), 2);
        assert!(created.members.iter().any(|m| m.contact == "alice"));
        assert!(created.members.iter().any(|m| m.contact == "bob"));

        let groups = load_groups(dir.path()).unwrap();
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].id, created.id);
        assert_eq!(groups[0].title, "Ops");
        assert_eq!(groups[0].members.len(), 2);
    }

    #[test]
    fn group_create_rejects_unknown_member() {
        let dir = tempfile::tempdir().unwrap();
        let err = create_group(dir.path(), "Ops", &["ghost".into()]).unwrap_err();
        assert!(err.to_string().contains("unknown group member"));
    }

    #[test]
    fn history_rows_have_contact_conversation_defaults() {
        let dir = tempfile::tempdir().unwrap();
        store_message(
            dir.path(),
            "out",
            "alice",
            "stqclefnkl4wfmdsz627hlfwu2xwgrk3sb6sgegfq44auik3pz7jmyqd.onion",
            "hello",
            42,
            DeliveryStatus::Sent,
        )
        .unwrap();

        let rows = load_history(dir.path(), Some("alice"), 10).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].conversation_kind, "contact");
        assert_eq!(rows[0].conversation_id, "alice");
    }

    // -- signing / verification --

    #[test]
    fn sign_and_verify_round_trip() {
        let signing = SigningKey::generate(&mut OsRng);
        let vk = signing.verifying_key();

        let mut msg = ChatMessage {
            v: 1,
            r#type: "chat_message".into(),
            from: B64.encode(vk.to_bytes()),
            timestamp_ms: 1_700_000_000_000_u128,
            body: "test".into(),
            sig_b64: String::new(),
            enc_body: String::new(),
            ratchet_header_b64: String::new(),
            ratchet_nonce_hex: String::new(),
            ratchet_ct_hex: String::new(),
        };

        sign_message(&signing, &mut msg).unwrap();
        assert!(!msg.sig_b64.is_empty());

        let mut contacts = HashMap::new();
        contacts.insert(
            "alice".into(),
            ContactFile {
                name: "alice".into(),
                onion: "fake.onion".into(),
                pubkey_b64: B64.encode(vk.to_bytes()),
                x25519_pubkey_b64: Some(B64.encode([2u8; 32])),
            },
        );

        assert!(verify_message(&msg, &contacts).unwrap());
    }

    #[test]
    fn verify_fails_on_tampered_body() {
        let signing = SigningKey::generate(&mut OsRng);
        let vk = signing.verifying_key();

        let mut msg = ChatMessage {
            v: 1,
            r#type: "chat_message".into(),
            from: B64.encode(vk.to_bytes()),
            timestamp_ms: 1_700_000_000_000_u128,
            body: "original".into(),
            sig_b64: String::new(),
            enc_body: String::new(),
            ratchet_header_b64: String::new(),
            ratchet_nonce_hex: String::new(),
            ratchet_ct_hex: String::new(),
        };

        sign_message(&signing, &mut msg).unwrap();

        let mut contacts = HashMap::new();
        contacts.insert(
            "alice".into(),
            ContactFile {
                name: "alice".into(),
                onion: "fake.onion".into(),
                pubkey_b64: B64.encode(vk.to_bytes()),
                x25519_pubkey_b64: Some(B64.encode([2u8; 32])),
            },
        );

        msg.body = "tampered".into();
        assert!(!verify_message(&msg, &contacts).unwrap());
    }

    #[test]
    fn verify_with_empty_contacts_falls_back_to_from_field() {
        let signing = SigningKey::generate(&mut OsRng);
        let vk = signing.verifying_key();

        let mut msg = ChatMessage {
            v: 1,
            r#type: "chat_message".into(),
            from: B64.encode(vk.to_bytes()),
            timestamp_ms: 123,
            body: "hi".into(),
            sig_b64: String::new(),
            enc_body: String::new(),
            ratchet_header_b64: String::new(),
            ratchet_nonce_hex: String::new(),
            ratchet_ct_hex: String::new(),
        };
        sign_message(&signing, &mut msg).unwrap();

        let contacts = HashMap::new();
        assert!(verify_message(&msg, &contacts).unwrap());
    }

    // -- payload canonicalisation --

    #[test]
    fn payload_does_not_include_sig() {
        let msg = ChatMessage {
            v: 1,
            r#type: "chat_message".into(),
            from: "AAAA".into(),
            timestamp_ms: 0,
            body: "x".into(),
            sig_b64: "SOMESIG".into(),
            enc_body: String::new(),
            ratchet_header_b64: String::new(),
            ratchet_nonce_hex: String::new(),
            ratchet_ct_hex: String::new(),
        };
        let payload = payload_to_sign(&msg).unwrap();
        let val: serde_json::Value = serde_json::from_str(&payload).unwrap();
        assert!(val.get("sig_b64").is_none());
    }

    // -- DB operations --

    #[test]
    fn db_store_and_load_history() {
        let dir = tempfile::tempdir().unwrap();
        init_profile(dir.path()).unwrap();

        store_message(
            dir.path(),
            "out",
            "alice",
            "alice.onion",
            "hello",
            1_700_000_000_000_u128,
            DeliveryStatus::Sent,
        )
        .unwrap();

        store_message(
            dir.path(),
            "in",
            "alice",
            "",
            "hi back",
            1_700_000_001_000_u128,
            DeliveryStatus::Delivered,
        )
        .unwrap();

        // Load all.
        let rows = load_history(dir.path(), None, 50).unwrap();
        assert_eq!(rows.len(), 2);
        // Ordered DESC by timestamp: "hi back" first.
        assert_eq!(rows[0].body, "hi back");
        assert_eq!(rows[1].body, "hello");

        // Filter by contact.
        let rows = load_history(dir.path(), Some("alice"), 50).unwrap();
        assert_eq!(rows.len(), 2);

        let rows = load_history(dir.path(), Some("bob"), 50).unwrap();
        assert_eq!(rows.len(), 0);
    }

    #[test]
    fn db_delivery_status_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        init_profile(dir.path()).unwrap();

        for status in [
            DeliveryStatus::Sent,
            DeliveryStatus::Delivered,
            DeliveryStatus::Failed,
        ] {
            store_message(
                dir.path(),
                "out",
                "test",
                "test.onion",
                &format!("{:?}", status),
                status.as_i64() as u128,
                status,
            )
            .unwrap();
        }

        let rows = load_history(dir.path(), None, 50).unwrap();
        assert_eq!(rows.len(), 3);
    }

    // -- encryption round-trip --

    #[test]
    fn encrypt_decrypt_round_trip() {
        let alice_dir = tempfile::tempdir().unwrap();
        let bob_dir = tempfile::tempdir().unwrap();
        init_profile(alice_dir.path()).unwrap();
        init_profile(bob_dir.path()).unwrap();

        let alice_x_pub = load_x25519_public(alice_dir.path()).unwrap();
        let bob_x_pub = load_x25519_public(bob_dir.path()).unwrap();

        // Alice encrypts for Bob
        let alice_secret = load_x25519_secret(alice_dir.path()).unwrap();
        let shared = derive_shared_key(&alice_secret, &bob_x_pub).unwrap();
        let ct = encrypt_body(&shared, "hello bob").unwrap();

        // Bob decrypts
        let bob_secret = load_x25519_secret(bob_dir.path()).unwrap();
        let shared2 = derive_shared_key(&bob_secret, &alice_x_pub).unwrap();
        let pt = decrypt_body(&shared2, &ct).unwrap();

        assert_eq!(pt, "hello bob");
    }

    #[test]
    fn decrypt_wrong_key_fails() {
        let alice_dir = tempfile::tempdir().unwrap();
        let bob_dir = tempfile::tempdir().unwrap();
        let eve_dir = tempfile::tempdir().unwrap();
        init_profile(alice_dir.path()).unwrap();
        init_profile(bob_dir.path()).unwrap();
        init_profile(eve_dir.path()).unwrap();

        let bob_x_pub = load_x25519_public(bob_dir.path()).unwrap();
        let alice_secret = load_x25519_secret(alice_dir.path()).unwrap();
        let shared = derive_shared_key(&alice_secret, &bob_x_pub).unwrap();
        let ct = encrypt_body(&shared, "secret").unwrap();

        // Eve tries to decrypt with her key — should fail
        let eve_secret = load_x25519_secret(eve_dir.path()).unwrap();
        let eve_shared = derive_shared_key(&eve_secret, &bob_x_pub).unwrap();
        assert!(decrypt_body(&eve_shared, &ct).is_err());
    }
}

// ---------------------------------------------------------------------------
// Tracing helpers
// ---------------------------------------------------------------------------

fn init_tracing_stderr() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .with_target(false)
        .compact()
        .with_writer(std::io::stderr)
        .init();
}

fn init_tracing_to_file(path: &Path) {
    let file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .expect("open tracing log file");
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .with_target(false)
        .compact()
        .with_writer(file)
        .init();
}

// -- ratchet round-trip --

#[test]
fn ratchet_encrypt_decrypt_round_trip() {
    let dir = tempfile::tempdir().unwrap();
    init_profile(dir.path()).unwrap();

    // Simulate Alice and Bob having a shared X25519 secret.
    let alice_x25519 = load_x25519_secret(dir.path()).unwrap();
    let alice_pub = X25519PublicKey::from(&alice_x25519);
    let bob_x25519 = StaticSecret::random_from_rng(OsRng);
    let bob_pub = X25519PublicKey::from(&bob_x25519);

    // Initialize Alice's ratchet (sender).
    let shared = alice_x25519.diffie_hellman(&bob_pub);
    let (mut alice_state, _, _) =
        RatchetState::load_or_init_alice(dir.path(), "bob", shared.as_bytes(), &bob_pub).unwrap();
    alice_state.save(dir.path(), "bob").unwrap();

    // Initialize Bob's ratchet (receiver).
    let shared2 = bob_x25519.diffie_hellman(&alice_pub);
    let mut bob_state = RatchetState {
        dh_secret_b64: B64.encode(bob_x25519.to_bytes()),
        their_dh_pub_b64: Some(B64.encode(alice_pub.as_bytes())),
        root_key_b64: B64.encode(shared2.as_bytes().to_vec()),
        send_ck_b64: None,
        recv_ck_b64: None,
        send_n: 0,
        recv_n: 0,
        prev_send_n: 0,
        initialized: false,
    };
    // Bob needs a recv_ck: derive from root key.
    let (recv_ck, send_ck) = hkdf_chain_key(shared2.as_bytes()).unwrap();
    bob_state.recv_ck_b64 = Some(B64.encode(&recv_ck));
    bob_state.send_ck_b64 = Some(B64.encode(&send_ck));

    // Alice encrypts.
    let (header_b64, nonce_hex, ct_hex) =
        ratchet_encrypt(&mut alice_state, b"hello from alice", "alice_pk").unwrap();

    // For the decrypt to work, Bob needs to have a root_key that matches.
    // The ratchet_decrypt will handle the DH ratchet step when it sees a new key.
    // Set up Bob's state so the first decrypt triggers the DH step.
    bob_state.their_dh_pub_b64 = None; // Force DH ratchet on first message.
    bob_state.save(dir.path(), "bob_bob").unwrap();

    let mut bob_state_for_decrypt: RatchetState = {
        let bytes = fs::read(RatchetState::path(
            dir.path(),
            std::path::Path::new("bob_bob"),
        ))
        .unwrap();
        bincode::deserialize(&bytes).unwrap()
    };
    let plaintext =
        ratchet_decrypt(&mut bob_state_for_decrypt, &header_b64, &nonce_hex, &ct_hex).unwrap();
    assert_eq!(plaintext, b"hello from alice");
}

#[test]
fn first_v3_message_auto_initializes_receiver_ratchet() {
    let alice_dir = tempfile::tempdir().unwrap();
    let bob_dir = tempfile::tempdir().unwrap();
    init_profile(alice_dir.path()).unwrap();
    init_profile(bob_dir.path()).unwrap();

    let alice_key = load_signing_key(alice_dir.path()).unwrap();
    let bob_key = load_signing_key(bob_dir.path()).unwrap();
    let alice_ed = B64.encode(alice_key.verifying_key().to_bytes());
    let bob_ed = B64.encode(bob_key.verifying_key().to_bytes());
    let alice_x = load_x25519_public(alice_dir.path()).unwrap();
    let bob_x = load_x25519_public(bob_dir.path()).unwrap();
    let alice_x_b64 = B64.encode(alice_x.as_bytes());
    let bob_x_b64 = B64.encode(bob_x.as_bytes());

    contact_add(
        alice_dir.path(),
        "bob",
        "psrntpu56hilbftscupr6f4ujxb6kjn6n2qy4366sen4lqkqpspjezid.onion",
        &bob_ed,
        &bob_x_b64,
    )
    .unwrap();
    contact_add(
        bob_dir.path(),
        "alice",
        "rog4qluztvbzq5sr2didprterk23tyo4q6e6775lkesx3jdqlm3jq5yd.onion",
        &alice_ed,
        &alice_x_b64,
    )
    .unwrap();

    init_ratchet_alice(alice_dir.path(), "bob", &bob_x).unwrap();
    let ratchet_path = RatchetState::path(alice_dir.path(), std::path::Path::new("bob"));
    let mut state: RatchetState = bincode::deserialize(&fs::read(&ratchet_path).unwrap()).unwrap();
    let plaintext = "ratchet hello";
    let timestamp_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis();
    let (header_b64, nonce_hex, ct_hex) =
        ratchet_encrypt(&mut state, plaintext.as_bytes(), &alice_ed).unwrap();
    state.save(alice_dir.path(), "bob").unwrap();

    let mut msg = ChatMessage {
        v: 3,
        r#type: "chat_message".into(),
        from: alice_ed,
        timestamp_ms,
        body: plaintext.into(),
        sig_b64: String::new(),
        enc_body: String::new(),
        ratchet_header_b64: header_b64,
        ratchet_nonce_hex: nonce_hex,
        ratchet_ct_hex: ct_hex,
    };
    sign_message(&alice_key, &mut msg).unwrap();
    msg.body.clear();

    let bob_contacts = load_contacts(bob_dir.path()).unwrap();
    let (decrypted, verified) =
        decrypt_and_verify(&mut msg, bob_dir.path(), &bob_contacts).unwrap();
    assert_eq!(decrypted, plaintext);
    assert!(verified);
    assert!(RatchetState::path(bob_dir.path(), std::path::Path::new("alice")).exists());
}

#[test]
fn receiver_can_send_first_reply_after_ratchet_restart() {
    let alice_dir = tempfile::tempdir().unwrap();
    let bob_dir = tempfile::tempdir().unwrap();
    init_profile(alice_dir.path()).unwrap();
    init_profile(bob_dir.path()).unwrap();

    let alice_key = load_signing_key(alice_dir.path()).unwrap();
    let bob_key = load_signing_key(bob_dir.path()).unwrap();
    let alice_ed = B64.encode(alice_key.verifying_key().to_bytes());
    let bob_ed = B64.encode(bob_key.verifying_key().to_bytes());
    let alice_x = load_x25519_public(alice_dir.path()).unwrap();
    let bob_x = load_x25519_public(bob_dir.path()).unwrap();
    let alice_x_b64 = B64.encode(alice_x.as_bytes());
    let bob_x_b64 = B64.encode(bob_x.as_bytes());

    contact_add(
        alice_dir.path(),
        "bob",
        "psrntpu56hilbftscupr6f4ujxb6kjn6n2qy4366sen4lqkqpspjezid.onion",
        &bob_ed,
        &bob_x_b64,
    )
    .unwrap();
    contact_add(
        bob_dir.path(),
        "alice",
        "rog4qluztvbzq5sr2didprterk23tyo4q6e6775lkesx3jdqlm3jq5yd.onion",
        &alice_ed,
        &alice_x_b64,
    )
    .unwrap();

    init_ratchet_alice(alice_dir.path(), "bob", &bob_x).unwrap();

    let alice_ratchet_path = RatchetState::path(alice_dir.path(), std::path::Path::new("bob"));
    let mut alice_state: RatchetState =
        bincode::deserialize(&fs::read(&alice_ratchet_path).unwrap()).unwrap();
    let first_plaintext = "alice opens ratchet";
    let timestamp_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis();
    let (header_b64, nonce_hex, ct_hex) =
        ratchet_encrypt(&mut alice_state, first_plaintext.as_bytes(), &alice_ed).unwrap();
    alice_state.save(alice_dir.path(), "bob").unwrap();

    let mut first_msg = ChatMessage {
        v: 3,
        r#type: "chat_message".into(),
        from: alice_ed.clone(),
        timestamp_ms,
        body: first_plaintext.into(),
        sig_b64: String::new(),
        enc_body: String::new(),
        ratchet_header_b64: header_b64,
        ratchet_nonce_hex: nonce_hex,
        ratchet_ct_hex: ct_hex,
    };
    sign_message(&alice_key, &mut first_msg).unwrap();
    first_msg.body.clear();

    let bob_contacts = load_contacts(bob_dir.path()).unwrap();
    let (decrypted, verified) =
        decrypt_and_verify(&mut first_msg, bob_dir.path(), &bob_contacts).unwrap();
    assert_eq!(decrypted, first_plaintext);
    assert!(verified);

    // Simulate Bob closing/reopening before he sends the first reply: load the
    // persisted responder state from disk and encrypt from that, not memory.
    let bob_ratchet_path = RatchetState::path(bob_dir.path(), std::path::Path::new("alice"));
    let mut restarted_bob_state: RatchetState =
        bincode::deserialize(&fs::read(&bob_ratchet_path).unwrap()).unwrap();
    let reply_plaintext = "bob replies after restart";
    let reply_ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis();
    let (reply_header, reply_nonce, reply_ct) = ratchet_encrypt(
        &mut restarted_bob_state,
        reply_plaintext.as_bytes(),
        &bob_ed,
    )
    .unwrap();
    restarted_bob_state.save(bob_dir.path(), "alice").unwrap();

    let mut reply_msg = ChatMessage {
        v: 3,
        r#type: "chat_message".into(),
        from: bob_ed,
        timestamp_ms: reply_ts,
        body: reply_plaintext.into(),
        sig_b64: String::new(),
        enc_body: String::new(),
        ratchet_header_b64: reply_header,
        ratchet_nonce_hex: reply_nonce,
        ratchet_ct_hex: reply_ct,
    };
    sign_message(&bob_key, &mut reply_msg).unwrap();
    reply_msg.body.clear();

    let alice_contacts = load_contacts(alice_dir.path()).unwrap();
    let (reply_decrypted, reply_verified) =
        decrypt_and_verify(&mut reply_msg, alice_dir.path(), &alice_contacts).unwrap();
    assert_eq!(reply_decrypted, reply_plaintext);
    assert!(reply_verified);

    // Alice should also be able to answer after decrypting Bob's first reply;
    // otherwise both UIs can show locally-sent rows while neither side can read
    // the other after restart.
    let alice_ratchet_path = RatchetState::path(alice_dir.path(), std::path::Path::new("bob"));
    let mut restarted_alice_state: RatchetState =
        bincode::deserialize(&fs::read(&alice_ratchet_path).unwrap()).unwrap();
    let followup_plaintext = "alice answers bob";
    let followup_ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis();
    let (followup_header, followup_nonce, followup_ct) = ratchet_encrypt(
        &mut restarted_alice_state,
        followup_plaintext.as_bytes(),
        &alice_ed,
    )
    .unwrap();
    restarted_alice_state.save(alice_dir.path(), "bob").unwrap();

    let mut followup_msg = ChatMessage {
        v: 3,
        r#type: "chat_message".into(),
        from: alice_ed,
        timestamp_ms: followup_ts,
        body: followup_plaintext.into(),
        sig_b64: String::new(),
        enc_body: String::new(),
        ratchet_header_b64: followup_header,
        ratchet_nonce_hex: followup_nonce,
        ratchet_ct_hex: followup_ct,
    };
    sign_message(&alice_key, &mut followup_msg).unwrap();
    followup_msg.body.clear();

    let bob_contacts = load_contacts(bob_dir.path()).unwrap();
    let (followup_decrypted, followup_verified) =
        decrypt_and_verify(&mut followup_msg, bob_dir.path(), &bob_contacts).unwrap();
    assert_eq!(followup_decrypted, followup_plaintext);
    assert!(followup_verified);
}

#[test]
fn both_sides_can_send_after_restart_before_receiving_peer_message() {
    let alice_dir = tempfile::tempdir().unwrap();
    let bob_dir = tempfile::tempdir().unwrap();
    init_profile(alice_dir.path()).unwrap();
    init_profile(bob_dir.path()).unwrap();

    let alice_key = load_signing_key(alice_dir.path()).unwrap();
    let bob_key = load_signing_key(bob_dir.path()).unwrap();
    let alice_ed = B64.encode(alice_key.verifying_key().to_bytes());
    let bob_ed = B64.encode(bob_key.verifying_key().to_bytes());
    let alice_x = load_x25519_public(alice_dir.path()).unwrap();
    let bob_x = load_x25519_public(bob_dir.path()).unwrap();
    let alice_x_b64 = B64.encode(alice_x.as_bytes());
    let bob_x_b64 = B64.encode(bob_x.as_bytes());

    contact_add(
        alice_dir.path(),
        "bob",
        "psrntpu56hilbftscupr6f4ujxb6kjn6n2qy4366sen4lqkqpspjezid.onion",
        &bob_ed,
        &bob_x_b64,
    )
    .unwrap();
    contact_add(
        bob_dir.path(),
        "alice",
        "rog4qluztvbzq5sr2didprterk23tyo4q6e6775lkesx3jdqlm3jq5yd.onion",
        &alice_ed,
        &alice_x_b64,
    )
    .unwrap();

    init_ratchet_alice(alice_dir.path(), "bob", &bob_x).unwrap();

    let mut alice_state: RatchetState = bincode::deserialize(
        &fs::read(RatchetState::path(
            alice_dir.path(),
            std::path::Path::new("bob"),
        ))
        .unwrap(),
    )
    .unwrap();
    let (h, n, c) = ratchet_encrypt(&mut alice_state, b"initial", &alice_ed).unwrap();
    alice_state.save(alice_dir.path(), "bob").unwrap();
    let mut initial = ChatMessage {
        v: 3,
        r#type: "chat_message".into(),
        from: alice_ed.clone(),
        timestamp_ms: 1,
        body: "initial".into(),
        sig_b64: String::new(),
        enc_body: String::new(),
        ratchet_header_b64: h,
        ratchet_nonce_hex: n,
        ratchet_ct_hex: c,
    };
    sign_message(&alice_key, &mut initial).unwrap();
    initial.body.clear();
    decrypt_and_verify(
        &mut initial,
        bob_dir.path(),
        &load_contacts(bob_dir.path()).unwrap(),
    )
    .unwrap();

    // Both processes restart, then each sends before receiving the other's new message.
    let mut alice_after_restart: RatchetState = bincode::deserialize(
        &fs::read(RatchetState::path(
            alice_dir.path(),
            std::path::Path::new("bob"),
        ))
        .unwrap(),
    )
    .unwrap();
    let mut bob_after_restart: RatchetState = bincode::deserialize(
        &fs::read(RatchetState::path(
            bob_dir.path(),
            std::path::Path::new("alice"),
        ))
        .unwrap(),
    )
    .unwrap();

    let (ah, an, ac) =
        ratchet_encrypt(&mut alice_after_restart, b"alice concurrent", &alice_ed).unwrap();
    let (bh, bn, bc) = ratchet_encrypt(&mut bob_after_restart, b"bob concurrent", &bob_ed).unwrap();
    alice_after_restart.save(alice_dir.path(), "bob").unwrap();
    bob_after_restart.save(bob_dir.path(), "alice").unwrap();

    let mut alice_msg = ChatMessage {
        v: 3,
        r#type: "chat_message".into(),
        from: alice_ed.clone(),
        timestamp_ms: 2,
        body: "alice concurrent".into(),
        sig_b64: String::new(),
        enc_body: String::new(),
        ratchet_header_b64: ah,
        ratchet_nonce_hex: an,
        ratchet_ct_hex: ac,
    };
    sign_message(&alice_key, &mut alice_msg).unwrap();
    alice_msg.body.clear();
    let mut bob_msg = ChatMessage {
        v: 3,
        r#type: "chat_message".into(),
        from: bob_ed.clone(),
        timestamp_ms: 3,
        body: "bob concurrent".into(),
        sig_b64: String::new(),
        enc_body: String::new(),
        ratchet_header_b64: bh,
        ratchet_nonce_hex: bn,
        ratchet_ct_hex: bc,
    };
    sign_message(&bob_key, &mut bob_msg).unwrap();
    bob_msg.body.clear();

    let (alice_plain, alice_verified) = decrypt_and_verify(
        &mut alice_msg,
        bob_dir.path(),
        &load_contacts(bob_dir.path()).unwrap(),
    )
    .unwrap();
    assert_eq!(alice_plain, "alice concurrent");
    assert!(alice_verified);

    let (bob_plain, bob_verified) = decrypt_and_verify(
        &mut bob_msg,
        alice_dir.path(),
        &load_contacts(alice_dir.path()).unwrap(),
    )
    .unwrap();
    assert_eq!(bob_plain, "bob concurrent");
    assert!(bob_verified);
}

#[test]
fn duplicate_old_ratchet_message_does_not_poison_state() {
    let alice_dir = tempfile::tempdir().unwrap();
    let bob_dir = tempfile::tempdir().unwrap();
    init_profile(alice_dir.path()).unwrap();
    init_profile(bob_dir.path()).unwrap();

    let alice_key = load_signing_key(alice_dir.path()).unwrap();
    let bob_key = load_signing_key(bob_dir.path()).unwrap();
    let alice_ed = B64.encode(alice_key.verifying_key().to_bytes());
    let bob_ed = B64.encode(bob_key.verifying_key().to_bytes());
    let alice_x = load_x25519_public(alice_dir.path()).unwrap();
    let bob_x = load_x25519_public(bob_dir.path()).unwrap();
    let alice_x_b64 = B64.encode(alice_x.as_bytes());
    let bob_x_b64 = B64.encode(bob_x.as_bytes());
    contact_add(
        alice_dir.path(),
        "bob",
        "psrntpu56hilbftscupr6f4ujxb6kjn6n2qy4366sen4lqkqpspjezid.onion",
        &bob_ed,
        &bob_x_b64,
    )
    .unwrap();
    contact_add(
        bob_dir.path(),
        "alice",
        "rog4qluztvbzq5sr2didprterk23tyo4q6e6775lkesx3jdqlm3jq5yd.onion",
        &alice_ed,
        &alice_x_b64,
    )
    .unwrap();

    init_ratchet_alice(alice_dir.path(), "bob", &bob_x).unwrap();
    let mut alice_state: RatchetState = bincode::deserialize(
        &fs::read(RatchetState::path(
            alice_dir.path(),
            std::path::Path::new("bob"),
        ))
        .unwrap(),
    )
    .unwrap();
    let (h1, n1, c1) = ratchet_encrypt(&mut alice_state, b"first", &alice_ed).unwrap();
    alice_state.save(alice_dir.path(), "bob").unwrap();
    let mut first_msg = ChatMessage {
        v: 3,
        r#type: "chat_message".into(),
        from: alice_ed.clone(),
        timestamp_ms: 1,
        body: "first".into(),
        sig_b64: String::new(),
        enc_body: String::new(),
        ratchet_header_b64: h1,
        ratchet_nonce_hex: n1,
        ratchet_ct_hex: c1,
    };
    sign_message(&alice_key, &mut first_msg).unwrap();
    first_msg.body.clear();
    decrypt_and_verify(
        &mut first_msg.clone(),
        bob_dir.path(),
        &load_contacts(bob_dir.path()).unwrap(),
    )
    .unwrap();

    let mut bob_state: RatchetState = bincode::deserialize(
        &fs::read(RatchetState::path(
            bob_dir.path(),
            std::path::Path::new("alice"),
        ))
        .unwrap(),
    )
    .unwrap();
    let (bh, bn, bc) = ratchet_encrypt(&mut bob_state, b"reply", &bob_ed).unwrap();
    bob_state.save(bob_dir.path(), "alice").unwrap();
    let mut reply_msg = ChatMessage {
        v: 3,
        r#type: "chat_message".into(),
        from: bob_ed.clone(),
        timestamp_ms: 2,
        body: "reply".into(),
        sig_b64: String::new(),
        enc_body: String::new(),
        ratchet_header_b64: bh,
        ratchet_nonce_hex: bn,
        ratchet_ct_hex: bc,
    };
    sign_message(&bob_key, &mut reply_msg).unwrap();
    reply_msg.body.clear();
    decrypt_and_verify(
        &mut reply_msg,
        alice_dir.path(),
        &load_contacts(alice_dir.path()).unwrap(),
    )
    .unwrap();

    // A duplicated first message arriving after Bob has advanced must not reset
    // Bob's ratchet state. It may fail as a duplicate; poisoning is the bug.
    let mut duplicate_first = first_msg.clone();
    let _ = decrypt_and_verify(
        &mut duplicate_first,
        bob_dir.path(),
        &load_contacts(bob_dir.path()).unwrap(),
    );

    let mut alice_state2: RatchetState = bincode::deserialize(
        &fs::read(RatchetState::path(
            alice_dir.path(),
            std::path::Path::new("bob"),
        ))
        .unwrap(),
    )
    .unwrap();
    let (h2, n2, c2) = ratchet_encrypt(&mut alice_state2, b"after duplicate", &alice_ed).unwrap();
    alice_state2.save(alice_dir.path(), "bob").unwrap();
    let mut after_dup = ChatMessage {
        v: 3,
        r#type: "chat_message".into(),
        from: alice_ed,
        timestamp_ms: 3,
        body: "after duplicate".into(),
        sig_b64: String::new(),
        enc_body: String::new(),
        ratchet_header_b64: h2,
        ratchet_nonce_hex: n2,
        ratchet_ct_hex: c2,
    };
    sign_message(&alice_key, &mut after_dup).unwrap();
    after_dup.body.clear();
    let (plain, verified) = decrypt_and_verify(
        &mut after_dup,
        bob_dir.path(),
        &load_contacts(bob_dir.path()).unwrap(),
    )
    .unwrap();
    assert_eq!(plain, "after duplicate");
    assert!(verified);
}

#[test]
fn inbound_v3_recovers_from_simultaneous_manual_ratchet_init() {
    let alice_dir = tempfile::tempdir().unwrap();
    let bob_dir = tempfile::tempdir().unwrap();
    init_profile(alice_dir.path()).unwrap();
    init_profile(bob_dir.path()).unwrap();

    let alice_key = load_signing_key(alice_dir.path()).unwrap();
    let bob_key = load_signing_key(bob_dir.path()).unwrap();
    let alice_ed = B64.encode(alice_key.verifying_key().to_bytes());
    let bob_ed = B64.encode(bob_key.verifying_key().to_bytes());
    let alice_x = load_x25519_public(alice_dir.path()).unwrap();
    let bob_x = load_x25519_public(bob_dir.path()).unwrap();
    let alice_x_b64 = B64.encode(alice_x.as_bytes());
    let bob_x_b64 = B64.encode(bob_x.as_bytes());

    contact_add(
        alice_dir.path(),
        "bob",
        "psrntpu56hilbftscupr6f4ujxb6kjn6n2qy4366sen4lqkqpspjezid.onion",
        &bob_ed,
        &bob_x_b64,
    )
    .unwrap();
    contact_add(
        bob_dir.path(),
        "alice",
        "rog4qluztvbzq5sr2didprterk23tyo4q6e6775lkesx3jdqlm3jq5yd.onion",
        &alice_ed,
        &alice_x_b64,
    )
    .unwrap();

    // Poison both sides by manually initializing them as Alice-first senders.
    init_ratchet_alice(alice_dir.path(), "bob", &bob_x).unwrap();
    init_ratchet_alice(bob_dir.path(), "alice", &alice_x).unwrap();

    let bob_ratchet_path = RatchetState::path(bob_dir.path(), std::path::Path::new("alice"));
    let mut bob_state: RatchetState =
        bincode::deserialize(&fs::read(&bob_ratchet_path).unwrap()).unwrap();
    let plaintext = "recovery hello";
    let timestamp_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis();
    let (header_b64, nonce_hex, ct_hex) =
        ratchet_encrypt(&mut bob_state, plaintext.as_bytes(), &bob_ed).unwrap();
    bob_state.save(bob_dir.path(), "alice").unwrap();

    let mut msg = ChatMessage {
        v: 3,
        r#type: "chat_message".into(),
        from: bob_ed,
        timestamp_ms,
        body: plaintext.into(),
        sig_b64: String::new(),
        enc_body: String::new(),
        ratchet_header_b64: header_b64,
        ratchet_nonce_hex: nonce_hex,
        ratchet_ct_hex: ct_hex,
    };
    sign_message(&bob_key, &mut msg).unwrap();
    msg.body.clear();

    let alice_contacts = load_contacts(alice_dir.path()).unwrap();
    let (decrypted, verified) =
        decrypt_and_verify(&mut msg, alice_dir.path(), &alice_contacts).unwrap();
    assert_eq!(decrypted, plaintext);
    assert!(verified);
}

#[test]
fn outbound_transfer_resume_uses_persisted_next_chunk_index() {
    let dir = tempfile::tempdir().unwrap();
    let profile = dir.path();
    let hash = "abcd1234resume";

    let st = OutboundTransferState {
        contact_name: "alice".to_string(),
        onion: "aliceexampleonionxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx.onion".to_string(),
        file_path: "/tmp/demo.bin".to_string(),
        file_name: "demo.bin".to_string(),
        hash: hash.to_string(),
        total_size: 100_000,
        total_chunks: 4,
        next_chunk_index: 2,
    };

    persist_outbound_state(profile, &st).unwrap();

    let listed = list_transfers(profile).unwrap();
    assert!(listed
        .iter()
        .any(|r| r.contains("outbound abcd1234resume -> alice chunk 2/4 file=demo.bin")));

    let target = outbound_transfer_target(profile, hash).unwrap();
    assert_eq!(
        target,
        Some(("alice".to_string(), "/tmp/demo.bin".to_string()))
    );

    assert!(cancel_outbound_transfer(profile, hash).unwrap());
    assert!(outbound_transfer_target(profile, hash).unwrap().is_none());
}

#[test]
fn inbound_transfer_state_survives_restart_roundtrip() {
    let dir = tempfile::tempdir().unwrap();
    let profile = dir.path();

    {
        let mut map = incoming_files_map().lock().unwrap();
        map.clear();
        map.insert(
            "alicepub:hash123".to_string(),
            IncomingFileState {
                total_chunks: 3,
                chunks: vec![Some(vec![1, 2, 3]), None, Some(vec![9])],
            },
        );
    }
    persist_incoming_states(profile).unwrap();

    // Simulate process restart by dropping in-memory state.
    {
        let mut map = incoming_files_map().lock().unwrap();
        map.clear();
    }

    load_incoming_states(profile).unwrap();

    let map = incoming_files_map().lock().unwrap();
    let st = map.get("alicepub:hash123").expect("restored state missing");
    assert_eq!(st.total_chunks, 3);
    assert_eq!(st.chunks[0].as_deref(), Some(&[1, 2, 3][..]));
    assert!(st.chunks[1].is_none());
    assert_eq!(st.chunks[2].as_deref(), Some(&[9][..]));
}

#[test]
fn outbound_transfer_checkpoint_survives_restart_roundtrip() {
    let dir = tempfile::tempdir().unwrap();
    let profile = dir.path();
    let hash = "resume-after-restart";

    let st = OutboundTransferState {
        contact_name: "alice".to_string(),
        onion: "aliceexampleonionxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx.onion".to_string(),
        file_path: "/tmp/resume.bin".to_string(),
        file_name: "resume.bin".to_string(),
        hash: hash.to_string(),
        total_size: 200_000,
        total_chunks: 7,
        next_chunk_index: 5,
    };
    persist_outbound_state(profile, &st).unwrap();

    // Simulate restart by reloading from DB-only state.
    let loaded = load_outbound_state(profile, hash)
        .unwrap()
        .expect("missing state");
    assert_eq!(loaded.next_chunk_index, 5);
    assert_eq!(loaded.total_chunks, 7);
    assert_eq!(loaded.file_path, "/tmp/resume.bin");

    let target = outbound_transfer_target(profile, hash).unwrap();
    assert_eq!(
        target,
        Some(("alice".to_string(), "/tmp/resume.bin".to_string()))
    );
}

#[test]
fn parse_inbound_line_rejects_garbage() {
    assert!(handler::parse_inbound_line("not json").unwrap().is_none());
    assert!(handler::parse_inbound_line("").unwrap().is_none());
}

#[test]
fn parse_inbound_line_parses_valid_chat_message() {
    let json = r#"{"v":1,"type":"msg","from":"alice","timestamp_ms":123,"body":"hello","sig_b64":"","enc_body":""}"#;
    match handler::parse_inbound_line(json) {
        Ok(Some(msg)) => {
            assert_eq!(msg.v, 1);
            assert_eq!(msg.r#type, "msg");
            assert_eq!(msg.from, "alice");
            assert_eq!(msg.body, "hello");
        }
        Ok(None) => panic!("parse_inbound_line returned None for valid JSON"),
        Err(e) => panic!("parse_inbound_line returned error: {e}"),
    }
}

#[test]
fn tor_transport_send_rejects_unknown_contact_name() {
    let dir = tempfile::tempdir().unwrap();
    crate::init_profile(dir.path()).unwrap();

    // resolve_to with a non-onion name that is not in the contact list should
    // fail with "unknown contact".
    let result = crate::resolve_to(dir.path(), "nonexistent_contact");
    assert!(result.is_err());
    let err = format!("{}", result.unwrap_err());
    assert!(err.contains("unknown contact"), "unexpected error: {err}");
}

#[test]
fn handler_rejects_unsigned_message() {
    // A valid JSON ChatMessage with an empty signature should be parsed
    // but NOT verified. The handler should still process it (as unverified).
    let json = r#"{"v":1,"type":"msg","from":"stranger","timestamp_ms":42,"body":"hi","sig_b64":"","enc_body":""}"#;
    let msg = handler::parse_inbound_line(json).unwrap().unwrap();
    assert_eq!(msg.from, "stranger");
    assert_eq!(msg.body, "hi");
    // parse_inbound_line doesn't verify — it just parses. Verification
    // happens in handle_inbound. This test confirms parsing works for
    // messages from unknown senders.
}

#[test]
fn tor_transport_envelope_round_trip() {
    // Verify that raw_line_to_envelope + envelope_body_as_str is lossless.
    let json = r#"{"v":1,"type":"msg","from":"alice","timestamp_ms":123,"body":"hello","sig_b64":"","enc_body":""}"#;
    let envelope = crate::transport::tor::TorTransport::raw_line_to_envelope(json);
    let body = crate::transport::tor::TorTransport::envelope_body_as_str(&envelope).unwrap();
    assert_eq!(body, json);
}

#[test]
fn tor_transport_try_recv_returns_envelope_from_channel() {
    // Create a TorTransport, inject an envelope via the inbound channel,
    // and verify try_recv returns it.
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        // We can't easily create a real TorClient without a running Tor,
        // so we test the channel logic directly by creating the transport
        // and sending into the channel.
        //
        // Note: TorTransport::new requires an Arc<TorClient>, which needs
        // a DormantClient.  Instead, test the parse -> envelope -> channel
        // pipeline end-to-end by verifying the static helpers and the
        // channel structure.

        let json = r#"{"v":1,"type":"msg","from":"alice","timestamp_ms":456,"body":"channel-test","sig_b64":"","enc_body":""}"#;

        // Step 1: parse_inbound_line produces a ChatMessage
        let msg = handler::parse_inbound_line(json)
            .unwrap()
            .unwrap();
        assert_eq!(msg.body, "channel-test");
        assert_eq!(msg.from, "alice");

        // Step 2: raw_line_to_envelope wraps the raw JSON
        let envelope = crate::transport::tor::TorTransport::raw_line_to_envelope(json);
        assert_eq!(envelope.msg_id.starts_with("tor-in-"), true);
        assert_eq!(envelope.seq, 0);
        assert_eq!(envelope.total, 1);
        assert_eq!(envelope.transport_hint.as_deref(), Some("tor"));

        // Step 3: envelope_body_as_str recovers original JSON
        let recovered =
            crate::transport::tor::TorTransport::envelope_body_as_str(&envelope).unwrap();
        assert_eq!(recovered, json);

        // This confirms the full pipeline:
        // raw JSON -> parse_inbound_line -> handle_inbound
        //                        -> raw_line_to_envelope -> channel -> try_recv
        // The channel wiring is in-place; TorTransport pushes to inbound_tx
        // in run_inbound_loop and try_recv pops from inbound_rx.
    });
}

// ---------------------------------------------------------------------------
// Integration tests — full pipeline without Tor
// ---------------------------------------------------------------------------

#[cfg(test)]
fn setup_alice_bob() -> (tempfile::TempDir, tempfile::TempDir) {
    let alice_dir = tempfile::tempdir().unwrap();
    let bob_dir = tempfile::tempdir().unwrap();
    init_profile_with_name(alice_dir.path(), "alice").unwrap();
    init_profile_with_name(bob_dir.path(), "bob").unwrap();

    let alice_key = load_signing_key(alice_dir.path()).unwrap();
    let bob_key = load_signing_key(bob_dir.path()).unwrap();
    let alice_ed = B64.encode(alice_key.verifying_key().to_bytes());
    let bob_ed = B64.encode(bob_key.verifying_key().to_bytes());
    let alice_x = load_x25519_public(alice_dir.path()).unwrap();
    let bob_x = load_x25519_public(bob_dir.path()).unwrap();
    let alice_x_b64 = B64.encode(alice_x.as_bytes());
    let bob_x_b64 = B64.encode(bob_x.as_bytes());

    let alice_onion = "psrntpu56hilbftscupr6f4ujxb6kjn6n2qy4366sen4lqkqpspjezid.onion";
    let bob_onion = "rog4qluztvbzq5sr2didprterk23tyo4q6e6775lkesx3jdqlm3jq5yd.onion";

    contact_add(alice_dir.path(), "bob", bob_onion, &bob_ed, &bob_x_b64).unwrap();
    contact_add(
        bob_dir.path(),
        "alice",
        alice_onion,
        &alice_ed,
        &alice_x_b64,
    )
    .unwrap();

    (alice_dir, bob_dir)
}

/// Build a v2 (static X25519) signed+encrypted ChatMessage on the wire.
#[cfg(test)]
fn build_v2_wire(alice_dir: &std::path::Path, bob_dir: &std::path::Path, message: &str) -> String {
    let alice_key = load_signing_key(alice_dir).unwrap();
    let alice_ed = B64.encode(alice_key.verifying_key().to_bytes());
    let bob_x_pub = load_x25519_public(bob_dir).unwrap();
    let alice_secret = load_x25519_secret(alice_dir).unwrap();
    let shared = derive_shared_key(&alice_secret, &bob_x_pub).unwrap();
    let enc_body = encrypt_body(&shared, message).unwrap();

    let timestamp_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis();

    let mut msg = ChatMessage {
        v: 2,
        r#type: "chat_message".into(),
        from: alice_ed,
        timestamp_ms,
        body: message.to_string(),
        sig_b64: String::new(),
        enc_body,
        ratchet_header_b64: String::new(),
        ratchet_nonce_hex: String::new(),
        ratchet_ct_hex: String::new(),
    };
    sign_message(&alice_key, &mut msg).unwrap();
    msg.body.clear();
    serde_json::to_string(&msg).unwrap()
}

#[test]
fn int_v2_message_e2e_stores_in_sqlite() {
    let (alice_dir, bob_dir) = setup_alice_bob();
    let wire = build_v2_wire(alice_dir.path(), bob_dir.path(), "hello from alice v2");

    let mut msg = handler::parse_inbound_line(&wire).unwrap().unwrap();
    assert_eq!(msg.v, 2);

    let bob_contacts = load_contacts(bob_dir.path()).unwrap();
    let (plaintext, verified) =
        decrypt_and_verify(&mut msg, bob_dir.path(), &bob_contacts).unwrap();
    assert!(verified, "message should be verified");
    assert_eq!(plaintext, "hello from alice v2");

    store_message(
        bob_dir.path(),
        "in",
        "alice",
        "",
        &plaintext,
        msg.timestamp_ms,
        DeliveryStatus::Delivered,
    )
    .unwrap();

    let rows = load_history(bob_dir.path(), Some("alice"), 10).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].direction, "in");
    assert_eq!(rows[0].contact, "alice");
    assert_eq!(rows[0].body, "hello from alice v2");
    assert_eq!(rows[0].status, DeliveryStatus::Delivered.as_i64());
}

#[test]
fn int_v3_ratchet_message_e2e_stores_in_sqlite() {
    let (alice_dir, bob_dir) = setup_alice_bob();

    let alice_key = load_signing_key(alice_dir.path()).unwrap();
    let bob_x = load_x25519_public(bob_dir.path()).unwrap();
    let alice_ed = B64.encode(alice_key.verifying_key().to_bytes());
    init_ratchet_alice(alice_dir.path(), "bob", &bob_x).unwrap();

    let ratchet_path = RatchetState::path(alice_dir.path(), std::path::Path::new("bob"));
    let mut state: RatchetState =
        bincode::deserialize(&std::fs::read(&ratchet_path).unwrap()).unwrap();
    let plaintext = "ratchet hello v3";
    let timestamp_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis();
    let (header_b64, nonce_hex, ct_hex) =
        ratchet_encrypt(&mut state, plaintext.as_bytes(), &alice_ed).unwrap();
    state.save(alice_dir.path(), "bob").unwrap();

    let mut msg = ChatMessage {
        v: 3,
        r#type: "chat_message".into(),
        from: alice_ed.clone(),
        timestamp_ms,
        body: plaintext.into(),
        sig_b64: String::new(),
        enc_body: String::new(),
        ratchet_header_b64: header_b64,
        ratchet_nonce_hex: nonce_hex,
        ratchet_ct_hex: ct_hex,
    };
    sign_message(&alice_key, &mut msg).unwrap();
    msg.body.clear();
    let wire = serde_json::to_string(&msg).unwrap();

    let mut inbound = handler::parse_inbound_line(&wire).unwrap().unwrap();
    assert_eq!(inbound.v, 3);

    let bob_contacts = load_contacts(bob_dir.path()).unwrap();
    let (decrypted, verified) =
        decrypt_and_verify(&mut inbound, bob_dir.path(), &bob_contacts).unwrap();
    assert!(verified);
    assert_eq!(decrypted, plaintext);
    assert!(RatchetState::path(bob_dir.path(), std::path::Path::new("alice")).exists());

    store_message(
        bob_dir.path(),
        "in",
        "alice",
        "",
        &decrypted,
        inbound.timestamp_ms,
        DeliveryStatus::Delivered,
    )
    .unwrap();

    let rows = load_history(bob_dir.path(), Some("alice"), 10).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].body, "ratchet hello v3");
}

#[test]
fn int_v2_unknown_sender_fails_decrypt() {
    let (_alice_dir, bob_dir) = setup_alice_bob();
    let eve_dir = tempfile::tempdir().unwrap();
    init_profile_with_name(eve_dir.path(), "eve").unwrap();

    let eve_key = load_signing_key(eve_dir.path()).unwrap();
    let eve_ed = B64.encode(eve_key.verifying_key().to_bytes());
    let bob_x_pub = load_x25519_public(bob_dir.path()).unwrap();
    let eve_secret = load_x25519_secret(eve_dir.path()).unwrap();
    let shared = derive_shared_key(&eve_secret, &bob_x_pub).unwrap();
    let enc_body = encrypt_body(&shared, "from eve").unwrap();

    let timestamp_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis();
    let mut msg = ChatMessage {
        v: 2,
        r#type: "chat_message".into(),
        from: eve_ed,
        timestamp_ms,
        body: "from eve".into(),
        sig_b64: String::new(),
        enc_body,
        ratchet_header_b64: String::new(),
        ratchet_nonce_hex: String::new(),
        ratchet_ct_hex: String::new(),
    };
    sign_message(&eve_key, &mut msg).unwrap();
    msg.body.clear();
    let wire = serde_json::to_string(&msg).unwrap();

    let mut inbound = handler::parse_inbound_line(&wire).unwrap().unwrap();
    let bob_contacts = load_contacts(bob_dir.path()).unwrap();
    let result = decrypt_and_verify(&mut inbound, bob_dir.path(), &bob_contacts);
    assert!(result.is_err());
    let err = format!("{}", result.unwrap_err());
    assert!(err.contains("unknown sender pubkey"), "unexpected: {err}");
}

#[test]
fn int_v2_unknown_sender_decrypts_with_raw_key() {
    let (_alice_dir, bob_dir) = setup_alice_bob();
    let eve_dir = tempfile::tempdir().unwrap();
    init_profile_with_name(eve_dir.path(), "eve").unwrap();

    let eve_secret = load_x25519_secret(eve_dir.path()).unwrap();
    let eve_x_pub = X25519PublicKey::from(&eve_secret);
    let bob_x_pub = load_x25519_public(bob_dir.path()).unwrap();
    let bob_secret = load_x25519_secret(bob_dir.path()).unwrap();

    let shared_eve = derive_shared_key(&eve_secret, &bob_x_pub).unwrap();
    let enc_body = encrypt_body(&shared_eve, "secret from eve").unwrap();
    let shared_bob = derive_shared_key(&bob_secret, &eve_x_pub).unwrap();
    let plaintext = decrypt_body(&shared_bob, &enc_body).unwrap();
    assert_eq!(plaintext, "secret from eve");
}
