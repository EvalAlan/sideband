use std::collections::HashMap;
use std::fs;
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
use futures::StreamExt;
use hkdf::Hkdf;
use rand::rngs::OsRng;
use rusqlite::{params, Connection};
use safelog::DisplayRedacted;
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
use tokio::time::sleep;
use tracing::{error, info, warn};
use x25519_dalek::{PublicKey as X25519PublicKey, StaticSecret};

use arti_client::{TorClient, TorClientConfig};
use tor_hsservice::config::OnionServiceConfigBuilder;
use tor_rtcompat::PreferredRuntime;

mod tui;

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
    Serve(ProfileArg),
    Send {
        #[command(flatten)]
        profile: ProfileArg,
        /// Onion address or contact name.
        #[arg(long)]
        to: String,
        #[arg(long)]
        message: String,
    },
    Contact {
        #[command(subcommand)]
        action: ContactAction,
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
    List {
        #[command(flatten)]
        profile: ProfileArg,
    },
}

// ---------------------------------------------------------------------------
// Data model
// ---------------------------------------------------------------------------

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
type ContactsMap = HashMap<String, ContactFile>;

/// Chat message format (v1 = signed plaintext, v2 = signed + encrypted, v3 = double ratchet).
/// In v2 the `body` field empty on wire, `enc_body` holds ChaCha20-Poly1305 ciphertext.
/// In v3 `body` and `enc_body` are empty; ratchet_header_b64, ratchet_nonce_hex,
/// and ratchet_ct_hex carry the Double Ratchet payload.
#[derive(Debug, Serialize, Deserialize, Clone)]
struct ChatMessage {
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

const FILE_CHUNK_SIZE: usize = 32 * 1024; // 32 KB chunks

/// Send a file to a contact. Chunks the file and sends each chunk as a file_chunk message.
/// First sends a file_offer, then waits for file_ack before sending chunks.
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
    let total_chunks = (total_size + FILE_CHUNK_SIZE - 1) / FILE_CHUNK_SIZE;

    // Resolve contact.
    let onion = crate::resolve_to(profile, contact_name)?;

    // Build and send file_offer message.
    let offer_payload = serde_json::json!({
        "type": "file_offer",
        "name": file_name,
        "size": total_size,
        "hash": hash,
        "total_chunks": total_chunks,
    });
    let offer_json = offer_payload.to_string();

    // Send the offer using the same encrypted channel.
    let key = crate::load_signing_key(profile)?;
    let our_ed25519_pub =
        base64::engine::general_purpose::STANDARD.encode(key.verifying_key().to_bytes());
    let timestamp_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_millis();

    let ratchet_path = RatchetState::path(profile, std::path::Path::new(contact_name));

    let offer_msg = if ratchet_path.exists() {
        let mut state_bytes = std::fs::read(&ratchet_path)?;
        let mut state: RatchetState = bincode::deserialize(&mut state_bytes)?;
        let (header_b64, nonce_hex, ct_hex) =
            ratchet_encrypt(&mut state, offer_json.as_bytes(), &our_ed25519_pub)?;
        state.save(profile, contact_name)?;
        let mut sign_msg = ChatMessage {
            v: 3,
            r#type: "file_offer".into(),
            from: our_ed25519_pub.clone(),
            timestamp_ms,
            body: String::new(),
            sig_b64: String::new(),
            enc_body: String::new(),
            ratchet_header_b64: header_b64,
            ratchet_nonce_hex: nonce_hex,
            ratchet_ct_hex: ct_hex,
        };
        crate::sign_message(&key, &mut sign_msg)?;
        sign_msg
    } else {
        let mut msg = ChatMessage {
            v: 2,
            r#type: "file_offer".into(),
            from: our_ed25519_pub.clone(),
            timestamp_ms,
            body: String::new(),
            sig_b64: String::new(),
            enc_body: String::new(),
            ratchet_header_b64: String::new(),
            ratchet_nonce_hex: String::new(),
            ratchet_ct_hex: String::new(),
        };
        crate::sign_message(&key, &mut msg)?;
        let our_x25519 = crate::load_x25519_secret(profile)?;
        let their_x25519 = crate::resolve_x25519_pubkey(profile, contact_name)?;
        let shared_key = crate::derive_shared_key(&our_x25519, &their_x25519)?;
        msg.enc_body = crate::encrypt_body(&shared_key, &offer_json)?;
        msg
    };

    // Send offer via Arti Tor client.
    let payload = format!("{}\n", serde_json::to_string(&offer_msg)?);

    let connect_result = {
        let onion = onion.clone();
        let payload = payload.clone();
        let tc = Arc::clone(&tor_client);
        let connect_fut = async move {
            let addr = format!("{}:80", onion);
            let mut stream = tc
                .connect(addr.as_str())
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
        tokio::time::timeout(std::time::Duration::from_secs(30), connect_fut).await
    };

    let send_ok = match connect_result {
        Ok(Ok(())) => {
            info!(
                "file offer sent: {} ({} bytes, {} chunks)",
                file_name, total_size, total_chunks
            );
            true
        }
        Ok(Err(e)) => {
            error!("file offer send error: {e}");
            false
        }
        Err(_) => {
            error!("file offer send timed out");
            false
        }
    };

    // Store outbound file offer in DB.
    crate::store_message(
        profile,
        "out",
        contact_name,
        &onion,
        &format!("[file offer: {} ({} bytes)]", file_name, total_size),
        timestamp_ms,
        if send_ok {
            crate::DeliveryStatus::Sent
        } else {
            crate::DeliveryStatus::Failed
        },
    )?;

    // Drop tor_client to shut down Arti cleanly.
    drop(tor_client);

    // Note: In a full implementation, we'd wait for the file_ack and then send chunks.
    // For now, we send the offer and the receiver can request chunks.
    // The chunk sending would be triggered by a file_request message.
    Ok(())
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
            ON messages(timestamp_ms);",
    )?;
    Ok(conn)
}

fn store_message(
    profile: &Path,
    direction: &str,
    contact: &str,
    onion: &str,
    body: &str,
    timestamp_ms: u128,
    status: DeliveryStatus,
) -> Result<()> {
    let conn = init_db(profile)?;
    conn.execute(
        "INSERT INTO messages (direction, contact, onion, body, timestamp_ms, status)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            direction,
            contact,
            onion,
            body,
            timestamp_ms as i64,
            status.as_i64()
        ],
    )?;
    Ok(())
}

#[allow(dead_code)]
struct HistoryRow {
    id: i64,
    direction: String,
    contact: String,
    onion: String,
    body: String,
    timestamp_ms: i64,
    status: i64,
    created_at: String,
}

fn load_history(
    profile: &Path,
    contact_filter: Option<&str>,
    limit: usize,
) -> Result<Vec<HistoryRow>> {
    let conn = init_db(profile)?;

    if let Some(c) = contact_filter {
        let mut stmt = conn.prepare(
            "SELECT id, direction, contact, onion, body, timestamp_ms, status, created_at
             FROM messages
             WHERE contact = ?1
             ORDER BY timestamp_ms DESC LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![c, limit as i64], |r| {
            Ok(HistoryRow {
                id: r.get(0)?,
                direction: r.get(1)?,
                contact: r.get(2)?,
                onion: r.get(3)?,
                body: r.get(4)?,
                timestamp_ms: r.get(5)?,
                status: r.get(6)?,
                created_at: r.get(7)?,
            })
        })?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    } else {
        let mut stmt = conn.prepare(
            "SELECT id, direction, contact, onion, body, timestamp_ms, status, created_at
             FROM messages
             ORDER BY timestamp_ms DESC LIMIT ?1",
        )?;
        let rows = stmt.query_map(params![limit as i64], |r| {
            Ok(HistoryRow {
                id: r.get(0)?,
                direction: r.get(1)?,
                contact: r.get(2)?,
                onion: r.get(3)?,
                body: r.get(4)?,
                timestamp_ms: r.get(5)?,
                status: r.get(6)?,
                created_at: r.get(7)?,
            })
        })?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }
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
        CommandKind::Serve(args) => {
            let profile = args.path()?;
            ensure_profile(&profile)?;
            let (tx, _rx) = mpsc::channel::<TuiEvent>(64);
            let (_quit_tx, quit_rx) = tokio::sync::oneshot::channel::<()>();
            let tor_client = create_tor_client(&profile).await?;
            serve(&profile, tx, quit_rx, Arc::new(tor_client)).await
        }
        CommandKind::Send {
            profile,
            to,
            message,
        } => {
            let profile = profile.path()?;
            ensure_profile(&profile)?;
            let onion = resolve_to(&profile, &to)?;
            let tor_client = Arc::new(create_tor_client(&profile).await?);
            send(&profile, &onion, &message, &to, None, tor_client).await
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
            ContactAction::List { profile } => {
                let profile = profile.path()?;
                ensure_profile(&profile)?;
                contact_list(&profile)
            }
        },
        CommandKind::History {
            profile,
            contact,
            limit,
        } => {
            let profile = profile.path()?;
            ensure_profile(&profile)?;
            history(&profile, contact.as_deref(), limit)
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

fn ensure_profile(profile: &Path) -> Result<()> {
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

    println!("\n  === Sideband First-Time Setup ===\n");
    println!("  Profile: {}", profile.display());
    println!("  Default display name: {default_name}\n");
    print!("  Enter your display name [{default_name}]: ");
    io::stdout().flush()?;

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

    println!("\n  Creating profile as '{name}'...\n");

    // Create profile with the chosen name
    init_profile_with_name(profile, &name)
}

fn init_profile_with_name(profile: &Path, display_name: &str) -> Result<()> {
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

fn contact_add(
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

fn contact_list(profile: &Path) -> Result<()> {
    let contacts = load_contacts(profile)?;
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
    let _msg_n = u32::from_be_bytes(header_bytes[32..36].try_into().unwrap());
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

    // Advance the receive chain to the right message number.
    // For simplicity, we advance one step per call (no skip buffer yet).
    let (next_ck, mk) = hkdf_chain_key(&ck_bytes)?;
    state.recv_ck_b64 = Some(B64.encode(&next_ck));
    state.recv_n += 1;

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
fn decrypt_and_verify(
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

/// Create a bootstrapped Arti Tor client.
/// State is stored under `<profile>/arti_state`.
async fn create_tor_client(profile: &Path) -> Result<TorClient<PreferredRuntime>> {
    let state_dir = profile.join("arti_state");
    fs::create_dir_all(&state_dir)?;

    let config = TorClientConfig::default();
    let tor_client = TorClient::create_bootstrapped(config)
        .await
        .context("failed to bootstrap Arti Tor client")?;
    Ok(tor_client)
}

// ---------------------------------------------------------------------------
// Serve
// ---------------------------------------------------------------------------

/// Format an HsId as a valid v3 .onion address.
fn hsid_to_onion(hsid: &tor_hsservice::HsId) -> String {
    hsid.display_unredacted().to_string()
}

async fn serve(
    profile: &Path,
    tui_tx: mpsc::Sender<TuiEvent>,
    mut quit_rx: tokio::sync::oneshot::Receiver<()>,
    tor_client: Arc<TorClient<PreferredRuntime>>,
) -> Result<()> {
    let _key = load_signing_key(profile)?;
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let listen_port = listener.local_addr()?.port();

    // Create an Arti onion service that forwards to our local listener.
    let nickname = tor_hsservice::HsNickname::new("sideband".into())
        .map_err(|e| anyhow!("invalid nickname: {e}"))?;
    let hs_config = OnionServiceConfigBuilder::default()
        .nickname(nickname)
        .build()
        .context("build onion service config")?;
    let (onion_svc, onion_request_stream) = tor_client
        .launch_onion_service(hs_config)
        .context("launch onion service")?
        .context("onion service disabled or failed to launch")?;
    let onion_hsid = onion_svc
        .onion_address()
        .context("onion service has no address — key may not be ready")?;
    let onion = hsid_to_onion(&onion_hsid);

    let _ = tui_tx
        .send(TuiEvent::StatusUpdate(format!("onion={onion}")))
        .await;
    info!(%onion, listen_port, "serve ready via Arti onion service");

    // Arti onion services do not forward streams automatically. We must
    // consume rendezvous requests, accept client streams, and bridge them to
    // Sideband's existing local TCP protocol listener.
    let local_addr = format!("127.0.0.1:{listen_port}");
    tokio::spawn(async move {
        let mut stream_requests = tor_hsservice::handle_rend_requests(onion_request_stream);
        while let Some(req) = stream_requests.next().await {
            let local_addr = local_addr.clone();
            tokio::spawn(async move {
                let mut onion_stream = match req
                    .accept(tor_cell::relaycell::msg::Connected::new_empty())
                    .await
                {
                    Ok(stream) => stream,
                    Err(e) => {
                        error!(error=%e, "failed to accept onion stream");
                        return;
                    }
                };

                let mut local_stream = match TcpStream::connect(&local_addr).await {
                    Ok(stream) => stream,
                    Err(e) => {
                        error!(error=%e, %local_addr, "failed to connect onion stream to local listener");
                        return;
                    }
                };

                if let Err(e) =
                    tokio::io::copy_bidirectional(&mut onion_stream, &mut local_stream).await
                {
                    error!(error=%e, "onion stream bridge failed");
                }
            });
        }
    });

    let contacts_for_spawn = load_contacts(profile).unwrap_or_default();

    loop {
        tokio::select! {
            incoming = listener.accept() => {
                let (stream, peer) = incoming?;
                let contacts = contacts_for_spawn.clone();
                let profile = profile.to_path_buf();
                let tui_tx = tui_tx.clone();
                info!(%peer, "incoming connection");
                tokio::spawn(async move {
                    let mut reader = BufReader::new(stream);
                    let mut line = String::new();
                    match reader.read_line(&mut line).await {
                        Ok(0) => {}
                        Ok(_) => {
                            if let Ok(mut msg) = serde_json::from_str::<ChatMessage>(line.trim()) {
                                // Handle file_offer messages specially.
                                if msg.r#type == "file_offer" {
                                    let (plaintext, verified) = decrypt_and_verify(&mut msg, &profile, &contacts).unwrap_or_else(|e| {
                                        error!(error=%e, "decrypt/verify failed");
                                        (String::new(), false)
                                    });
                                    let contact_name = contacts
                                        .values()
                                        .find(|c| c.pubkey_b64 == msg.from)
                                        .map(|c| c.name.clone())
                                        .unwrap_or_else(|| {
                                            if verified { "verified-peer".into() } else { msg.from.clone() }
                                        });
                                    let body_for_display = if verified && !plaintext.is_empty() {
                                        format!("[file offer] {}", plaintext)
                                    } else if verified {
                                        "[file offer received]".to_string()
                                    } else {
                                        "[file offer — UNVERIFIED]".to_string()
                                    };
                                    if let Err(e) = store_message(
                                        &profile, "in", &contact_name, "",
                                        &body_for_display, msg.timestamp_ms,
                                        if verified { DeliveryStatus::Delivered } else { DeliveryStatus::Failed },
                                    ) {
                                        error!(error=%e, "failed to store file offer");
                                    }
                                    let _ = tui_tx.send(TuiEvent::InboundMessage {
                                        contact: contact_name,
                                        body: body_for_display,
                                        timestamp_ms: msg.timestamp_ms,
                                        verified,
                                    }).await;
                                    info!(recv=true, %msg.r#type, "file offer received");
                                    return;
                                }

                                let decrypt_result = decrypt_and_verify(&mut msg, &profile, &contacts);
                                let decrypt_error = decrypt_result.as_ref().err().map(|e| e.to_string());
                                let (plaintext, verified) = decrypt_result.unwrap_or_else(|e| {
                                    error!(error=%e, "decrypt/verify failed");
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

                                let contact_name = contacts
                                    .values()
                                    .find(|c| c.pubkey_b64 == msg.from)
                                    .map(|c| c.name.clone())
                                    .unwrap_or_else(|| {
                                        if verified { "verified-peer".into() } else { msg.from.clone()
                                        }
                                    });

                                if let Err(e) = store_message(
                                    &profile, "in", &contact_name, "",
                                    &plaintext, msg.timestamp_ms,
                                    if verified { DeliveryStatus::Delivered } else { DeliveryStatus::Failed },
                                ) {
                                    error!(error=%e, "failed to store inbound message");
                                }

                                let _ = tui_tx.send(TuiEvent::InboundMessage {
                                    contact: contact_name.clone(),
                                    body: body_for_display,
                                    timestamp_ms: msg.timestamp_ms,
                                    verified,
                                }).await;

                                if verified {
                                    info!(recv=true, v=%msg.v, "message received and verified");
                                } else {
                                    warn!(from=%msg.from, "signature verification FAILED");
                                }
                            } else {
                                error!(raw=%line, "invalid inbound payload");
                            }
                        }
                        Err(e) => error!(error=%e, "read error"),
                    }
                });
            }
            _ = &mut quit_rx => {
                info!("serve received quit signal, shutting down");
                // Drop onion_svc + tor_client to shut down Arti cleanly.
                drop(onion_svc);
                drop(tor_client);
                return Ok(());
            }
        }
    }
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
    let use_ratchet = ratchet_path.exists();

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

fn history(profile: &Path, contact: Option<&str>, limit: usize) -> Result<()> {
    let rows = load_history(profile, contact, limit)?;
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
