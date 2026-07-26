//! Android RFCOMM carrier bridge.
//!
//! Rust owns routing, BTP, replay state, and application dispatch. The Android
//! adapter is a byte-pump reached over a private Unix-domain socket in the app
//! sandbox; no Dart/native callback is required.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use rand::RngCore;

use serde::{Deserialize, Serialize};
use tokio::io::{
    AsyncBufRead, AsyncBufReadExt, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader,
};
use tokio::sync::{mpsc, oneshot};

use super::Envelope;

const BRIDGE_QUEUE: usize = 32;
const BRIDGE_LINE_MAX: usize = 6 * 1024 * 1024;
#[cfg(not(test))]
const BRIDGE_TIMEOUT: Duration = Duration::from_secs(20);
#[cfg(test)]
const BRIDGE_TIMEOUT: Duration = Duration::from_millis(250);
const DEVICE_MAX: usize = 128;
const UUID_LEN: usize = 36;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct BluetoothProperty {
    pub v: u8,
    pub device: String,
    pub service_uuid: String,
}

impl BluetoothProperty {
    pub(crate) fn parse(value: &str) -> Result<Self> {
        let property: Self = serde_json::from_str(value).context("parse Bluetooth property")?;
        if property.v != 1 || !valid_device(&property.device) || !valid_uuid(&property.service_uuid)
        {
            bail!("invalid Bluetooth transport property");
        }
        Ok(property)
    }
}

fn valid_device(value: &str) -> bool {
    !value.trim().is_empty() && value.len() <= DEVICE_MAX && value.chars().all(|c| !c.is_control())
}

fn valid_uuid(value: &str) -> bool {
    value.len() == UUID_LEN
        && value.bytes().enumerate().all(|(i, b)| match i {
            8 | 13 | 18 | 23 => b == b'-',
            _ => b.is_ascii_hexdigit(),
        })
}

// ── Briar-style rotating service UUID ───────────────────────────────────────
//
// The legacy scheme advertises a random STATIC uuid that has to reach a contact
// out-of-band (transport_props over Tor/LAN, or the share code). That makes
// offline-first Bluetooth impossible, and exposes a stable identifier anyone
// running an SDP scan can use to recognise this device forever.
//
// Briar's approach, adopted here: derive the advertised uuid from the ACCOUNT
// KEY plus the current time epoch. A contact — who already holds our public key
// — recomputes the same uuid and connects to it; nobody else can attribute it,
// and it rotates. Combined with an SDP scan of nearby devices this removes the
// need to know a peer's address at all: discover, match the uuid, learn the
// address.

/// How often the advertised service uuid rotates.
pub(crate) const BT_UUID_EPOCH_SECS: u64 = 900; // 15 minutes

const BT_UUID_DOMAIN: &[u8] = b"sideband-bt-service-uuid-v1";

/// The epoch an absolute time falls in.
pub(crate) fn bt_uuid_epoch(now_ms: u128) -> u64 {
    (now_ms / 1000 / BT_UUID_EPOCH_SECS as u128) as u64
}

fn uuid_from_bytes(bytes: &[u8]) -> String {
    let mut b = [0u8; 16];
    b.copy_from_slice(&bytes[..16]);
    // Shape it as a valid RFC-4122 v4 uuid so Android's parser accepts it.
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    format!(
        "{}-{}-{}-{}-{}",
        hex::encode(&b[0..4]),
        hex::encode(&b[4..6]),
        hex::encode(&b[6..8]),
        hex::encode(&b[8..10]),
        hex::encode(&b[10..16])
    )
}

/// The RFCOMM service uuid a device with `account_pubkey_b64` advertises during
/// `epoch`. Deterministic, so a contact can compute it from the public key alone.
pub(crate) fn rotating_service_uuid(account_pubkey_b64: &str, epoch: u64) -> String {
    use sha2::Digest;
    let mut hasher = sha2::Sha256::new();
    hasher.update(BT_UUID_DOMAIN);
    hasher.update(account_pubkey_b64.as_bytes());
    hasher.update(epoch.to_be_bytes());
    uuid_from_bytes(&hasher.finalize()[..16])
}

/// Match a service uuid discovered on a nearby device against known account
/// keys, tolerating a one-epoch skew (clock drift / rotation boundary). Returns
/// the matching key — i.e. "that device over there is this contact".
pub(crate) fn match_service_uuid<'a, I>(candidates: I, uuid: &str, now_ms: u128) -> Option<String>
where
    I: IntoIterator<Item = &'a str>,
{
    let epoch = bt_uuid_epoch(now_ms);
    let candidates: Vec<&str> = candidates.into_iter().collect();
    for probe in [epoch, epoch.saturating_sub(1), epoch.saturating_add(1)] {
        for key in &candidates {
            if rotating_service_uuid(key, probe).eq_ignore_ascii_case(uuid) {
                return Some((*key).to_string());
            }
        }
    }
    None
}

const SERVICE_UUID_KEY: &str = "bluetooth_service_uuid";
const LOCAL_DEVICE_KEY: &str = "bluetooth_local_device";

pub(crate) fn service_uuid(profile: &Path) -> Result<String> {
    if let Some(value) = crate::get_setting(profile, SERVICE_UUID_KEY)? {
        if valid_uuid(&value) {
            return Ok(value);
        }
    }
    let mut bytes = [0u8; 16];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let value = format!(
        "{}-{}-{}-{}-{}",
        hex::encode(&bytes[0..4]),
        hex::encode(&bytes[4..6]),
        hex::encode(&bytes[6..8]),
        hex::encode(&bytes[8..10]),
        hex::encode(&bytes[10..16])
    );
    crate::set_setting(profile, SERVICE_UUID_KEY, &value)?;
    Ok(value)
}

pub(crate) fn set_local_device(profile: &Path, device: &str) -> Result<()> {
    if !valid_device(device) {
        bail!("invalid Bluetooth device hint");
    }
    crate::set_setting(profile, LOCAL_DEVICE_KEY, device)
}

pub(crate) fn clear_local_device(profile: &Path) -> Result<()> {
    crate::set_setting(profile, LOCAL_DEVICE_KEY, "")
}

pub(crate) fn local_property(profile: &Path) -> Result<Option<BluetoothProperty>> {
    if !crate::bluetooth_enabled(profile) {
        return Ok(None);
    }
    let Some(device) = crate::get_setting(profile, LOCAL_DEVICE_KEY)? else {
        return Ok(None);
    };
    if !valid_device(&device) {
        return Ok(None);
    }
    Ok(Some(BluetoothProperty {
        v: 1,
        device,
        service_uuid: service_uuid(profile)?,
    }))
}

#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum BridgeCommand {
    Dial {
        id: u64,
        session_id: String,
        device: String,
        uuid: String,
    },
    Write {
        id: u64,
        session_id: String,
        wire_b64: String,
    },
    Ack {
        session_id: String,
        wire_b64: String,
    },
    Cancel {
        id: u64,
        session_id: String,
    },
    Close {
        session_id: String,
    },
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum BridgeEvent {
    SendResult {
        id: u64,
        ok: bool,
        #[serde(default)]
        error: String,
    },
    Inbound {
        session_id: String,
        wire_b64: String,
    },
}

enum BridgeRequest {
    Tracked {
        command: BridgeCommand,
        response: oneshot::Sender<Result<(), String>>,
    },
    Fire(BridgeCommand),
}

static BRIDGES: LazyLock<Mutex<HashMap<PathBuf, mpsc::Sender<BridgeRequest>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
static OUTBOUND_LOCKS: LazyLock<Mutex<HashMap<PathBuf, Arc<tokio::sync::Mutex<()>>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
type AckKey = (PathBuf, [u8; crate::transport::btp::ACK_LEN]);
static ACK_WAITERS: LazyLock<Mutex<HashMap<AckKey, oneshot::Sender<()>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

fn profile_key(profile: &Path) -> PathBuf {
    profile
        .canonicalize()
        .unwrap_or_else(|_| profile.to_path_buf())
}

fn outbound_lock(profile: &Path) -> Arc<tokio::sync::Mutex<()>> {
    let key = profile_key(profile);
    let mut locks = OUTBOUND_LOCKS.lock().expect("Bluetooth lock map poisoned");
    locks
        .entry(key)
        .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
        .clone()
}

async fn bridge_request(profile: &Path, command: BridgeCommand) -> Result<()> {
    let sender = BRIDGES
        .lock()
        .map_err(|_| anyhow!("Bluetooth bridge state poisoned"))?
        .get(&profile_key(profile))
        .cloned()
        .ok_or_else(|| anyhow!("Bluetooth bridge is not connected"))?;
    let (id, session_id) = match &command {
        BridgeCommand::Dial { id, session_id, .. }
        | BridgeCommand::Write { id, session_id, .. } => (*id, session_id.clone()),
        _ => bail!("untracked Bluetooth command used as request"),
    };
    let (response_tx, response_rx) = oneshot::channel();
    sender
        .send(BridgeRequest::Tracked {
            command,
            response: response_tx,
        })
        .await
        .map_err(|_| anyhow!("Bluetooth bridge disconnected"))?;
    match tokio::time::timeout(BRIDGE_TIMEOUT, response_rx).await {
        Ok(result) => result
            .map_err(|_| anyhow!("Bluetooth bridge dropped response"))?
            .map_err(|e| anyhow!("Bluetooth carrier failed: {e}")),
        Err(_) => {
            let _ = sender
                .send(BridgeRequest::Fire(BridgeCommand::Cancel {
                    id,
                    session_id,
                }))
                .await;
            Err(anyhow!("Bluetooth bridge request timed out"))
        }
    }
}

async fn bridge_fire(profile: &Path, command: BridgeCommand) {
    let sender = BRIDGES
        .lock()
        .ok()
        .and_then(|bridges| bridges.get(&profile_key(profile)).cloned());
    if let Some(sender) = sender {
        let _ = sender.send(BridgeRequest::Fire(command)).await;
    }
}

/// Dial a peer through the platform adapter, then reserve and write exactly one
/// BTP stream. Reserving after `dial` avoids burning replay-window counters while
/// the device is absent.
pub(crate) async fn send_btp(
    profile: &Path,
    contact_name: &str,
    property: &BluetoothProperty,
    payload: &str,
) -> Result<()> {
    let (peer, crypto) = crate::btp_contact_crypto(profile, contact_name)?;
    let lock = outbound_lock(profile);
    let _guard = lock.lock().await;
    let session_id = format!("out-{}", next_id());

    bridge_request(
        profile,
        BridgeCommand::Dial {
            id: next_id(),
            session_id: session_id.clone(),
            device: property.device.clone(),
            uuid: property.service_uuid.clone(),
        },
    )
    .await?;

    let stream_no = crate::btp_reserve_outbound_stream(profile, &peer)?;
    let material = crate::transport::btp::derive_stream_material(
        &crypto.root,
        crypto.send_direction,
        crate::transport::btp::current_period(),
        stream_no,
        crate::transport::btp::random_stream_salt(),
    )?;
    let wire = crate::transport::btp::encode_stream(
        &material,
        payload.as_bytes(),
        crate::transport::btp::PaddingPolicy::Bucketed,
    )?;
    let expected_ack = crate::transport::btp::acknowledgement(&material)?;
    let ack_key = (profile_key(profile), expected_ack);
    let (ack_tx, ack_rx) = oneshot::channel();
    ACK_WAITERS
        .lock()
        .map_err(|_| anyhow!("Bluetooth acknowledgement state poisoned"))?
        .insert(ack_key.clone(), ack_tx);
    let write_result = bridge_request(
        profile,
        BridgeCommand::Write {
            id: next_id(),
            session_id: session_id.clone(),
            wire_b64: B64.encode(wire),
        },
    )
    .await;
    if let Err(error) = write_result {
        ACK_WAITERS
            .lock()
            .map_err(|_| anyhow!("Bluetooth acknowledgement state poisoned"))?
            .remove(&ack_key);
        bridge_fire(
            profile,
            BridgeCommand::Close {
                session_id: session_id.clone(),
            },
        )
        .await;
        return Err(error);
    }
    let acknowledged = tokio::time::timeout(BRIDGE_TIMEOUT, ack_rx).await;
    ACK_WAITERS
        .lock()
        .map_err(|_| anyhow!("Bluetooth acknowledgement state poisoned"))?
        .remove(&ack_key);
    let result = match acknowledged {
        Ok(waiter) => waiter.map_err(|_| anyhow!("Bluetooth acknowledgement waiter dropped")),
        Err(_) => Err(anyhow!("Bluetooth BTP acknowledgement timed out")),
    };
    bridge_fire(profile, BridgeCommand::Close { session_id }).await;
    result
}

fn next_id() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

fn bluetooth_envelope(payload: Vec<u8>) -> Envelope {
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    Envelope {
        msg_id: format!("bluetooth-in-{now_ms}"),
        from: String::new(),
        to: String::new(),
        body: payload,
        seq: 0,
        total: 1,
        ttl: 1,
        hop_count: 0,
        transport_hint: Some("bluetooth".to_string()),
        ack_for: None,
    }
}

async fn read_bounded_line<R: AsyncBufRead + Unpin>(reader: &mut R) -> Result<Option<Vec<u8>>> {
    let mut line = Vec::new();
    let read = reader
        .take((BRIDGE_LINE_MAX + 1) as u64)
        .read_until(b'\n', &mut line)
        .await?;
    if read == 0 {
        return Ok(None);
    }
    if line.len() > BRIDGE_LINE_MAX || line.last() != Some(&b'\n') {
        bail!("oversized or unterminated Bluetooth bridge record");
    }
    line.pop();
    Ok(Some(line))
}

async fn run_connection<S>(
    profile: PathBuf,
    stream: S,
    mut requests: mpsc::Receiver<BridgeRequest>,
    inbound_tx: mpsc::Sender<Envelope>,
) -> Result<()>
where
    S: tokio::io::AsyncRead + AsyncWrite + Unpin,
{
    let (read_half, mut write_half) = tokio::io::split(stream);
    let mut reader = BufReader::new(read_half);
    let mut pending: HashMap<u64, oneshot::Sender<Result<(), String>>> = HashMap::new();

    loop {
        tokio::select! {
            request = requests.recv() => {
                let Some(request) = request else { return Ok(()); };
                let command = match request {
                    BridgeRequest::Tracked { command, response } => {
                        let id = match &command {
                            BridgeCommand::Dial { id, .. } | BridgeCommand::Write { id, .. } => *id,
                            _ => bail!("untracked command cannot expect a response"),
                        };
                        if pending.insert(id, response).is_some() {
                            bail!("duplicate Bluetooth bridge request id");
                        }
                        command
                    }
                    BridgeRequest::Fire(command) => command,
                };
                let mut encoded = serde_json::to_vec(&command)?;
                encoded.push(b'\n');
                write_half.write_all(&encoded).await?;
                write_half.flush().await?;
            }
            line = read_bounded_line(&mut reader) => {
                let Some(line) = line? else { return Ok(()); };
                match serde_json::from_slice::<BridgeEvent>(&line)? {
                    BridgeEvent::SendResult { id, ok, error } => {
                        let Some(response) = pending.remove(&id) else {
                            bail!("unexpected Bluetooth bridge response");
                        };
                        let _ = response.send(if ok { Ok(()) } else { Err(error) });
                    }
                    BridgeEvent::Inbound { session_id, wire_b64 } => {
                        let wire = match B64.decode(wire_b64) {
                            Ok(wire) => wire,
                            Err(_) => {
                                let mut encoded = serde_json::to_vec(&BridgeCommand::Close { session_id })?;
                                encoded.push(b'\n');
                                write_half.write_all(&encoded).await?;
                                write_half.flush().await?;
                                continue;
                            }
                        };
                        if wire.len() == crate::transport::btp::ACK_LEN {
                            let ack: [u8; crate::transport::btp::ACK_LEN] = wire
                                .try_into()
                                .map_err(|_| anyhow!("invalid Bluetooth acknowledgement"))?;
                            let waiter = ACK_WAITERS
                                .lock()
                                .map_err(|_| anyhow!("Bluetooth acknowledgement state poisoned"))?
                                .remove(&(profile_key(&profile), ack));
                            if let Some(waiter) = waiter {
                                let _ = waiter.send(());
                            } else {
                                let mut encoded = serde_json::to_vec(&BridgeCommand::Close { session_id })?;
                                encoded.push(b'\n');
                                write_half.write_all(&encoded).await?;
                                write_half.flush().await?;
                            }
                            continue;
                        }
                        let command = match crate::transport::btp::decode_authenticated_stream(&profile, &wire) {
                            Ok(Some(authenticated)) => {
                                if inbound_tx.try_send(bluetooth_envelope(authenticated.payload)).is_ok() {
                                    BridgeCommand::Ack {
                                        session_id,
                                        wire_b64: B64.encode(authenticated.acknowledgement),
                                    }
                                } else {
                                    BridgeCommand::Close { session_id }
                                }
                            }
                            _ => BridgeCommand::Close { session_id },
                        };
                        let mut encoded = serde_json::to_vec(&command)?;
                        encoded.push(b'\n');
                        write_half.write_all(&encoded).await?;
                        write_half.flush().await?;
                    }
                }
            }
        }
    }
}

pub(crate) fn socket_path(profile: &Path) -> PathBuf {
    profile.join("bluetooth.sock")
}

#[derive(Serialize)]
pub(crate) struct BridgeConfig {
    pub enabled: bool,
    pub socket_path: String,
    pub service_uuid: String,
}

pub(crate) fn bridge_config(profile: &Path) -> Result<BridgeConfig> {
    Ok(BridgeConfig {
        enabled: crate::bluetooth_enabled(profile),
        socket_path: socket_path(profile).to_string_lossy().into_owned(),
        service_uuid: service_uuid(profile)?,
    })
}

#[cfg(unix)]
pub(crate) async fn spawn_bridge_server(
    profile: PathBuf,
    inbound_tx: mpsc::Sender<Envelope>,
) -> Result<(PathBuf, tokio::task::JoinHandle<()>)> {
    use std::os::unix::fs::PermissionsExt;
    use tokio::net::UnixListener;

    let path = socket_path(&profile);
    match std::fs::remove_file(&path) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e.into()),
    }
    let listener = UnixListener::bind(&path)?;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
    let key = profile_key(&profile);
    let task_path = path.clone();
    let handle = tokio::spawn(async move {
        loop {
            let Ok((stream, _)) = listener.accept().await else {
                break;
            };
            let (request_tx, request_rx) = mpsc::channel(BRIDGE_QUEUE);
            if let Ok(mut bridges) = BRIDGES.lock() {
                bridges.insert(key.clone(), request_tx);
            }
            if let Err(e) =
                run_connection(profile.clone(), stream, request_rx, inbound_tx.clone()).await
            {
                tracing::debug!(error=%e, "Bluetooth platform bridge disconnected");
            }
            if let Ok(mut bridges) = BRIDGES.lock() {
                bridges.remove(&key);
            }
        }
        let _ = std::fs::remove_file(task_path);
    });
    Ok((path, handle))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{contact_add, init_profile_with_name, load_signing_key, load_x25519_secret};

    fn connect(a: &Path, a_name: &str, a_onion: &str, b: &Path, b_name: &str, b_onion: &str) {
        let a_ed = B64.encode(load_signing_key(a).unwrap().verifying_key().to_bytes());
        let a_x =
            B64.encode(x25519_dalek::PublicKey::from(&load_x25519_secret(a).unwrap()).as_bytes());
        let b_ed = B64.encode(load_signing_key(b).unwrap().verifying_key().to_bytes());
        let b_x =
            B64.encode(x25519_dalek::PublicKey::from(&load_x25519_secret(b).unwrap()).as_bytes());
        contact_add(a, b_name, b_onion, &b_ed, &b_x).unwrap();
        contact_add(b, a_name, a_onion, &a_ed, &a_x).unwrap();
    }

    #[test]
    fn property_validation_is_versioned_and_bounded() {
        let value = r#"{"v":1,"device":"AA:BB:CC:DD:EE:FF","service_uuid":"12345678-1234-5678-9abc-def012345678"}"#;
        assert_eq!(BluetoothProperty::parse(value).unwrap().v, 1);
        assert!(BluetoothProperty::parse(&value.replace("\"v\":1", "\"v\":2")).is_err());
        assert!(BluetoothProperty::parse(
            &value.replace("12345678-1234-5678-9abc-def012345678", "bad")
        )
        .is_err());
    }

    #[test]
    fn rotating_service_uuid_is_recognised_by_contacts_and_rotates() {
        let alice = "ALICE_ACCOUNT_PUBKEY";
        let bob = "BOB_ACCOUNT_PUBKEY";
        let now = 1_700_000_000_000u128;
        let epoch_ms = (BT_UUID_EPOCH_SECS as u128) * 1000;

        // Alice advertises; Bob, who holds her key, recomputes and recognises it
        // with no address exchange and nothing shared out of band.
        let advertised = rotating_service_uuid(alice, bt_uuid_epoch(now));
        assert!(valid_uuid(&advertised), "must be a well-formed uuid");
        assert_eq!(
            match_service_uuid([alice, bob], &advertised, now),
            Some(alice.to_string())
        );

        // Someone who does not hold Alice's key learns nothing from the advert.
        assert_eq!(match_service_uuid([bob], &advertised, now), None);

        // It rotates, so a device cannot be tracked across epochs by its uuid.
        let later = now + epoch_ms * 5;
        assert_ne!(
            advertised,
            rotating_service_uuid(alice, bt_uuid_epoch(later))
        );
        assert_eq!(match_service_uuid([alice], &advertised, later), None);
    }

    #[test]
    fn rotating_service_uuid_tolerates_one_epoch_of_clock_skew() {
        let alice = "ALICE_ACCOUNT_PUBKEY";
        let now = 1_700_000_000_000u128;
        let epoch = bt_uuid_epoch(now);
        // Two phones whose clocks differ, or an advert minted either side of a
        // rotation boundary, must still find each other.
        for probe in [epoch - 1, epoch + 1] {
            let advertised = rotating_service_uuid(alice, probe);
            assert_eq!(
                match_service_uuid([alice], &advertised, now),
                Some(alice.to_string()),
                "uuid from epoch {probe} should match at epoch {epoch}"
            );
        }
    }

    #[test]
    fn profile_uuid_and_advertised_property_are_stable_and_opt_in() {
        let profile = tempfile::tempdir().unwrap();
        init_profile_with_name(profile.path(), "alice").unwrap();
        assert!(!crate::bluetooth_enabled(profile.path()));
        assert!(local_property(profile.path()).unwrap().is_none());

        let first_uuid = service_uuid(profile.path()).unwrap();
        assert!(valid_uuid(&first_uuid));
        assert_eq!(service_uuid(profile.path()).unwrap(), first_uuid);

        crate::set_bluetooth_enabled(profile.path(), true).unwrap();
        assert!(local_property(profile.path()).unwrap().is_none());
        set_local_device(profile.path(), "name:Alice Phone").unwrap();
        let property = local_property(profile.path()).unwrap().unwrap();
        assert_eq!(property.device, "name:Alice Phone");
        assert_eq!(property.service_uuid, first_uuid);
        let encoded = serde_json::to_string(&property).unwrap();
        assert_eq!(BluetoothProperty::parse(&encoded).unwrap(), property);

        crate::set_bluetooth_enabled(profile.path(), false).unwrap();
        assert!(!crate::bluetooth_enabled(profile.path()));
        assert!(local_property(profile.path()).unwrap().is_none());
        crate::set_bluetooth_enabled(profile.path(), true).unwrap();
        assert!(local_property(profile.path()).unwrap().is_none());
    }

    #[tokio::test]
    async fn fake_platform_bridge_carries_only_btp_wire() {
        let alice_dir = tempfile::tempdir().unwrap();
        let bob_dir = tempfile::tempdir().unwrap();
        init_profile_with_name(alice_dir.path(), "alice").unwrap();
        init_profile_with_name(bob_dir.path(), "bob").unwrap();
        connect(
            alice_dir.path(),
            "alice",
            "fpmansl7byak6gq7ymzi7j3dvetjoi6i3oh2yt4tv5y5wgdnn2icuhid.onion",
            bob_dir.path(),
            "bob",
            "qg3dpwh42ldnuy2z42ldce5bc4g6pfpew4s3qti6bngp3hwfrtuvmoqd.onion",
        );

        let (core, platform) = tokio::io::duplex(8 * 1024 * 1024);
        let (request_tx, request_rx) = mpsc::channel(BRIDGE_QUEUE);
        BRIDGES
            .lock()
            .unwrap()
            .insert(profile_key(alice_dir.path()), request_tx);
        let (inbound_tx, _inbound_rx) = mpsc::channel(1);
        let runner = tokio::spawn(run_connection(
            alice_dir.path().to_path_buf(),
            core,
            request_rx,
            inbound_tx,
        ));
        let message = crate::build_outbound_message(
            alice_dir.path(),
            "bob",
            "msg",
            "bridge-secret",
            "fpmansl7byak6gq7ymzi7j3dvetjoi6i3oh2yt4tv5y5wgdnn2icuhid.onion",
            None,
        )
        .unwrap();
        let expected_payload = format!("{}\n", serde_json::to_string(&message).unwrap());
        let expected_by_platform = expected_payload.clone();
        let bob_profile = bob_dir.path().to_path_buf();
        let platform_task = tokio::spawn(async move {
            let (read, mut write) = tokio::io::split(platform);
            let mut reader = BufReader::new(read);
            let dial = read_bounded_line(&mut reader).await.unwrap().unwrap();
            let dial: serde_json::Value = serde_json::from_slice(&dial).unwrap();
            let id = dial["id"].as_u64().unwrap();
            let session_id = dial["session_id"].as_str().unwrap().to_string();
            write
                .write_all(
                    format!("{{\"type\":\"send_result\",\"id\":{id},\"ok\":true}}\n").as_bytes(),
                )
                .await
                .unwrap();
            let send = read_bounded_line(&mut reader).await.unwrap().unwrap();
            let send: serde_json::Value = serde_json::from_slice(&send).unwrap();
            assert_eq!(send["session_id"].as_str().unwrap(), session_id);
            let wire = B64.decode(send["wire_b64"].as_str().unwrap()).unwrap();
            assert!(!wire
                .windows(b"bridge-secret".len())
                .any(|w| w == b"bridge-secret"));
            let authenticated =
                crate::transport::btp::decode_authenticated_stream(&bob_profile, &wire)
                    .unwrap()
                    .unwrap();
            assert_eq!(authenticated.payload, expected_by_platform.as_bytes());
            let id = send["id"].as_u64().unwrap();
            write
                .write_all(
                    format!("{{\"type\":\"send_result\",\"id\":{id},\"ok\":true}}\n").as_bytes(),
                )
                .await
                .unwrap();
            write
                .write_all(
                    format!(
                        "{{\"type\":\"inbound\",\"session_id\":\"{}\",\"wire_b64\":\"{}\"}}\n",
                        session_id,
                        B64.encode(authenticated.acknowledgement),
                    )
                    .as_bytes(),
                )
                .await
                .unwrap();
        });
        let property = BluetoothProperty::parse(r#"{"v":1,"device":"AA:BB:CC:DD:EE:FF","service_uuid":"12345678-1234-5678-9abc-def012345678"}"#).unwrap();
        send_btp(alice_dir.path(), "bob", &property, &expected_payload)
            .await
            .unwrap();
        platform_task.await.unwrap();
        runner.abort();
        BRIDGES
            .lock()
            .unwrap()
            .remove(&profile_key(alice_dir.path()));
    }

    #[tokio::test]
    async fn timed_out_platform_request_does_not_wedge_bridge() {
        let profile = tempfile::tempdir().unwrap();
        let (core, platform) = tokio::io::duplex(64 * 1024);
        let (request_tx, request_rx) = mpsc::channel(8);
        let (inbound_tx, _inbound_rx) = mpsc::channel(1);
        BRIDGES
            .lock()
            .unwrap()
            .insert(profile_key(profile.path()), request_tx);
        let runner = tokio::spawn(run_connection(
            profile.path().to_path_buf(),
            core,
            request_rx,
            inbound_tx,
        ));
        let platform_task = tokio::spawn(async move {
            let (read, mut write) = tokio::io::split(platform);
            let mut reader = BufReader::new(read);
            let first: serde_json::Value =
                serde_json::from_slice(&read_bounded_line(&mut reader).await.unwrap().unwrap())
                    .unwrap();
            let first_id = first["id"].as_u64().unwrap();
            let cancel: serde_json::Value =
                serde_json::from_slice(&read_bounded_line(&mut reader).await.unwrap().unwrap())
                    .unwrap();
            assert_eq!(cancel["type"], "cancel");
            assert_eq!(cancel["id"], first_id);
            write
                .write_all(
                    format!("{{\"type\":\"send_result\",\"id\":{first_id},\"ok\":false,\"error\":\"cancelled\"}}\n").as_bytes(),
                )
                .await
                .unwrap();
            let second: serde_json::Value =
                serde_json::from_slice(&read_bounded_line(&mut reader).await.unwrap().unwrap())
                    .unwrap();
            let second_id = second["id"].as_u64().unwrap();
            write
                .write_all(
                    format!("{{\"type\":\"send_result\",\"id\":{second_id},\"ok\":true}}\n")
                        .as_bytes(),
                )
                .await
                .unwrap();
        });
        let command = |session_id: &str| BridgeCommand::Dial {
            id: next_id(),
            session_id: session_id.to_string(),
            device: session_id.to_string(),
            uuid: "12345678-1234-5678-9abc-def012345678".to_string(),
        };
        assert!(bridge_request(profile.path(), command("first"))
            .await
            .is_err());
        bridge_request(profile.path(), command("second"))
            .await
            .unwrap();
        platform_task.await.unwrap();
        runner.abort();
        BRIDGES.lock().unwrap().remove(&profile_key(profile.path()));
    }
}
