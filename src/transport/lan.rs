//! LAN transport, the first non-Tor carrier.
//!
//! Every TCP stream is wrapped by BTP-lite before it leaves the process. The
//! wire exposes only a masked authenticated stream counter, rotating 16-byte contact
//! tag, and padded ciphertext; the
//! encrypted payload is still a signed and end-to-end encrypted
//! [`crate::ChatMessage`]. Unknown tags, plaintext, failed authentication, and
//! replays are dropped. Valid frames receive a keyed carrier acknowledgement;
//! Tor remains the fallback when that acknowledgement is absent.
//!
//! The primary address-discovery path is a signed and encrypted
//! `transport_props` message shared only with accepted contacts. The optional
//! UDP helper broadcasts per-contact rotating tokens derived from static X25519
//! shared material. It never serializes a raw identity key.

use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, bail, Result};
use base64::{engine::general_purpose::STANDARD as B64, Engine};
use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use hkdf::Hkdf;
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream, UdpSocket};

use super::Envelope;

/// UDP port peers broadcast presence beacons on.
pub const LAN_DISCOVERY_PORT: u16 = 47654;
/// A discovered peer stays reachable for this long after its most recent beacon.
const PEER_TTL_MS: u128 = 90_000;
/// How often we broadcast our own beacon.
const BEACON_INTERVAL: Duration = Duration::from_secs(20);
/// Reject beacons whose clock is this far from ours (replay / stale guard).
const BEACON_MAX_SKEW_MS: u128 = 120_000;
/// Contact tokens rotate every 30 seconds. Receivers tolerate clock skew through
/// the signed timestamp freshness window, not by accepting an unbounded period.
const BEACON_PERIOD_MS: u128 = 30_000;
const MAX_BTP_WIRE: u64 = (crate::transport::btp::PREFIX_LEN
    + crate::transport::btp::HEADER_CIPHERTEXT_LEN
    + crate::transport::btp::MAX_STREAM_CONTENT
    + 16) as u64;
const MAX_CONCURRENT_BTP_STREAMS: usize = 32;
/// Timeout for a single LAN TCP send.
const LAN_SEND_TIMEOUT: Duration = Duration::from_secs(5);

fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

// ---------------------------------------------------------------------------
// Beacon
// ---------------------------------------------------------------------------

/// A signed, contact-recognizable presence beacon. `token_b64` is a rotating PRF
/// output known only to one contact; the sender's identity is never serialized.
/// The sender's IP is taken from the UDP packet, not the beacon.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LanBeacon {
    pub token_b64: String,
    pub tcp_port: u16,
    pub period: u64,
    pub timestamp_ms: u128,
    pub sig_b64: String,
}

fn beacon_period(timestamp_ms: u128) -> u64 {
    (timestamp_ms / BEACON_PERIOD_MS).min(u64::MAX as u128) as u64
}

/// Derive one contact's unlinkable token for a time period. The all-zero DH
/// result is rejected instead of turning an invalid X25519 key into a shared
/// public token.
pub(crate) fn contact_token(shared_secret: &[u8; 32], period: u64) -> Result<[u8; 16]> {
    if shared_secret.iter().all(|byte| *byte == 0) {
        return Err(anyhow!("invalid all-zero LAN contact secret"));
    }
    let hk = Hkdf::<Sha256>::new(Some(b"sideband/lan-beacon/v2/token-salt"), shared_secret);
    let mut info = b"sideband/lan-beacon/v2/contact-token".to_vec();
    info.extend_from_slice(&period.to_be_bytes());
    let mut token = [0u8; 16];
    hk.expand(&info, &mut token)
        .map_err(|_| anyhow!("derive LAN contact token"))?;
    Ok(token)
}

impl LanBeacon {
    /// Canonical bytes covered by the signature (everything but the signature).
    fn signing_bytes(token_b64: &str, tcp_port: u16, period: u64, timestamp_ms: u128) -> Vec<u8> {
        format!("sideband-lan-beacon-v2|{token_b64}|{tcp_port}|{period}|{timestamp_ms}")
            .into_bytes()
    }

    /// Build one contact-specific beacon advertising `tcp_port`.
    pub fn create(key: &SigningKey, token: [u8; 16], tcp_port: u16, period: u64) -> Self {
        let timestamp_ms = now_ms();
        Self::create_at(key, token, tcp_port, period, timestamp_ms)
    }

    fn create_at(
        key: &SigningKey,
        token: [u8; 16],
        tcp_port: u16,
        period: u64,
        timestamp_ms: u128,
    ) -> Self {
        let token_b64 = B64.encode(token);
        let sig = key.sign(&Self::signing_bytes(
            &token_b64,
            tcp_port,
            period,
            timestamp_ms,
        ));
        LanBeacon {
            token_b64,
            tcp_port,
            period,
            timestamp_ms,
            sig_b64: B64.encode(sig.to_bytes()),
        }
    }

    /// Verify against a candidate contact identity selected by token matching.
    pub fn verify(&self, verifying_key: &VerifyingKey) -> bool {
        let Ok(token) = B64.decode(&self.token_b64) else {
            return false;
        };
        if token.len() != 16 || self.period != beacon_period(self.timestamp_ms) {
            return false;
        }
        let Ok(sig_bytes) = B64.decode(&self.sig_b64) else {
            return false;
        };
        let Ok(sig_arr) = <[u8; 64]>::try_from(sig_bytes.as_slice()) else {
            return false;
        };
        let sig = Signature::from_bytes(&sig_arr);
        let msg = Self::signing_bytes(
            &self.token_b64,
            self.tcp_port,
            self.period,
            self.timestamp_ms,
        );
        verifying_key.verify_strict(&msg, &sig).is_ok()
    }

    /// True if the beacon is signed correctly AND its clock is within the skew
    /// window (rejects replays of very old beacons).
    pub fn is_valid_now(&self, verifying_key: &VerifyingKey) -> bool {
        if !self.verify(verifying_key) {
            return false;
        }
        let now = now_ms();
        let skew = now.abs_diff(self.timestamp_ms);
        skew <= BEACON_MAX_SKEW_MS
    }
}

// ---------------------------------------------------------------------------
// Discovered-peer registry
// ---------------------------------------------------------------------------

/// LAN addresses of identities we have heard beacons from, keyed by Ed25519 b64.
#[derive(Default)]
pub struct DiscoveredPeers {
    inner: Mutex<HashMap<String, (SocketAddr, u128)>>,
}

impl DiscoveredPeers {
    /// Record (or refresh) an identity's LAN address as of `seen_ms`.
    pub fn note(&self, ed25519_b64: &str, addr: SocketAddr, seen_ms: u128) {
        if let Ok(mut map) = self.inner.lock() {
            map.insert(ed25519_b64.to_string(), (addr, seen_ms));
        }
    }

    /// A still-fresh LAN address for `ed25519_b64`, or `None` if unknown/stale.
    pub fn lookup(&self, ed25519_b64: &str) -> Option<SocketAddr> {
        let map = self.inner.lock().ok()?;
        let (addr, seen) = map.get(ed25519_b64)?;
        if now_ms().saturating_sub(*seen) <= PEER_TTL_MS {
            Some(*addr)
        } else {
            None
        }
    }

    /// Number of peers seen within the freshness window.
    pub fn fresh_count(&self) -> usize {
        let now = now_ms();
        self.inner
            .lock()
            .map(|m| {
                m.values()
                    .filter(|(_, seen)| now.saturating_sub(*seen) <= PEER_TTL_MS)
                    .count()
            })
            .unwrap_or(0)
    }
}

/// Process-wide registry: the discovery loop writes it, the send path reads it.
pub static PEERS: LazyLock<DiscoveredPeers> = LazyLock::new(DiscoveredPeers::default);
static OUTBOUND_BTP_LOCKS: LazyLock<Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

fn outbound_btp_lock(profile: &std::path::Path, peer: &str) -> Arc<tokio::sync::Mutex<()>> {
    let key = format!("{}\0{peer}", profile.display());
    OUTBOUND_BTP_LOCKS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .entry(key)
        .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
        .clone()
}

/// Contact material needed by the opt-in dynamic-IP beacon helper. This never
/// goes on the wire: only a period token derived from `shared_secret` does.
#[derive(Clone)]
pub(crate) struct LanBeaconContact {
    pub ed25519_b64: String,
    pub shared_secret: [u8; 32],
}

impl LanBeaconContact {
    fn verifying_key(&self) -> Option<VerifyingKey> {
        let bytes = B64.decode(&self.ed25519_b64).ok()?;
        let bytes = <[u8; 32]>::try_from(bytes.as_slice()).ok()?;
        VerifyingKey::from_bytes(&bytes).ok()
    }
}

// ---------------------------------------------------------------------------
// TCP carrier
// ---------------------------------------------------------------------------

/// Wrap and send one authenticated BTP-lite stream. One connection per peer
/// direction is active at a time. The stream number is reserved after TCP
/// connect succeeds and before any encrypted bytes are built or written.
pub async fn send_btp(
    profile: &std::path::Path,
    contact_name: &str,
    addr: SocketAddr,
    payload: &str,
) -> Result<()> {
    let (peer, crypto) = crate::btp_contact_crypto(profile, contact_name)?;
    let lock = outbound_btp_lock(profile, &peer);
    let _guard = lock.lock().await;
    let fut = async {
        let mut stream = TcpStream::connect(addr)
            .await
            .map_err(|e| anyhow!("lan connect {addr}: {e}"))?;
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
        stream
            .write_all(&wire)
            .await
            .map_err(|e| anyhow!("lan BTP write: {e}"))?;
        stream
            .shutdown()
            .await
            .map_err(|e| anyhow!("lan BTP shutdown: {e}"))?;
        let mut ack = [0u8; crate::transport::btp::ACK_LEN];
        stream
            .read_exact(&mut ack)
            .await
            .map_err(|e| anyhow!("lan BTP acknowledgement: {e}"))?;
        if ack != expected_ack {
            bail!("invalid LAN BTP acknowledgement");
        }
        Ok(())
    };
    tokio::time::timeout(LAN_SEND_TIMEOUT, fut)
        .await
        .map_err(|_| anyhow!("lan BTP send to {addr} timed out"))?
}

fn lan_payload_to_envelope(payload: &[u8]) -> Envelope {
    Envelope {
        msg_id: format!("lan-in-{}", now_ms()),
        from: String::new(),
        to: String::new(),
        body: payload.to_vec(),
        seq: 0,
        total: 1,
        ttl: 1,
        hop_count: 0,
        transport_hint: Some("lan".to_string()),
        ack_for: None,
    }
}

/// Bind a TCP listener for inbound BTP-lite streams. Unknown tags, legacy
/// plaintext, failed authentication, replays, and oversized streams are dropped
/// without a protocol response.
///
/// Returns the bound port and the accept-loop task handle.
pub async fn spawn_listener(
    profile: std::path::PathBuf,
    inbound_tx: tokio::sync::mpsc::Sender<Envelope>,
) -> Result<(u16, tokio::task::JoinHandle<()>)> {
    let listener = TcpListener::bind((Ipv4Addr::UNSPECIFIED, 0)).await?;
    let port = listener.local_addr()?.port();
    let stream_slots = Arc::new(tokio::sync::Semaphore::new(MAX_CONCURRENT_BTP_STREAMS));
    let handle = tokio::spawn(async move {
        loop {
            let (stream, peer) = match listener.accept().await {
                Ok(v) => v,
                Err(e) => {
                    tracing::warn!(error=%e, "lan listener accept failed");
                    continue;
                }
            };
            let Ok(stream_slot) = stream_slots.clone().try_acquire_owned() else {
                continue;
            };
            let inbound_tx = inbound_tx.clone();
            let profile = profile.clone();
            tokio::spawn(async move {
                let _stream_slot = stream_slot;
                let mut stream = stream;
                let mut wire = vec![0u8; crate::transport::btp::PREFIX_AND_HEADER_LEN];
                if !matches!(
                    tokio::time::timeout(Duration::from_secs(15), stream.read_exact(&mut wire))
                        .await,
                    Ok(Ok(_))
                ) {
                    return;
                }
                let Ok(Some(frame_len)) =
                    crate::transport::btp::authenticated_wire_len(&profile, &wire)
                else {
                    return;
                };
                if frame_len as u64 > MAX_BTP_WIRE {
                    return;
                }
                let prefix_len = wire.len();
                wire.resize(frame_len, 0);
                if !matches!(
                    tokio::time::timeout(
                        Duration::from_secs(15),
                        stream.read_exact(&mut wire[prefix_len..]),
                    )
                    .await,
                    Ok(Ok(_))
                ) {
                    return;
                }
                let Ok(Some(authenticated)) =
                    crate::transport::btp::decode_authenticated_stream(&profile, &wire)
                else {
                    return;
                };
                if inbound_tx
                    .send(lan_payload_to_envelope(&authenticated.payload))
                    .await
                    .is_err()
                {
                    return;
                }
                if stream
                    .write_all(&authenticated.acknowledgement)
                    .await
                    .is_err()
                {
                    return;
                }
                tracing::debug!(%peer, "accepted authenticated LAN BTP stream");
            });
        }
    });
    Ok((port, handle))
}

// ---------------------------------------------------------------------------
// UDP-broadcast discovery
// ---------------------------------------------------------------------------

/// Start LAN discovery: broadcast one rotating token per contact and record the
/// LAN address of a contact whose token and signature we recognize. `contacts`
/// is refreshed for every interval/packet so contact changes do not require a
/// listener restart.
///
/// Beacons are written into the process-wide [`PEERS`] registry.
pub async fn spawn_discovery(
    key: SigningKey,
    tcp_port: u16,
    contacts: impl Fn() -> Vec<LanBeaconContact> + Send + Sync + 'static,
) -> Result<()> {
    let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, LAN_DISCOVERY_PORT))
        .await
        .map_err(|e| anyhow!("bind lan discovery udp: {e}"))?;
    socket
        .set_broadcast(true)
        .map_err(|e| anyhow!("enable udp broadcast: {e}"))?;
    let socket = std::sync::Arc::new(socket);
    let contacts = std::sync::Arc::new(contacts);

    // Broadcaster.
    let bcast_socket = std::sync::Arc::clone(&socket);
    let bcast_key = key.clone();
    let bcast_contacts = std::sync::Arc::clone(&contacts);
    tokio::spawn(async move {
        let dst = SocketAddr::new(IpAddr::V4(Ipv4Addr::BROADCAST), LAN_DISCOVERY_PORT);
        loop {
            let period = beacon_period(now_ms());
            for contact in bcast_contacts() {
                let Ok(token) = contact_token(&contact.shared_secret, period) else {
                    continue;
                };
                let beacon = LanBeacon::create(&bcast_key, token, tcp_port, period);
                if let Ok(json) = serde_json::to_vec(&beacon) {
                    if let Err(e) = bcast_socket.send_to(&json, dst).await {
                        tracing::debug!(error=%e, "lan beacon broadcast failed");
                    }
                }
            }
            tokio::time::sleep(BEACON_INTERVAL).await;
        }
    });

    // Listener.
    tokio::spawn(async move {
        let mut buf = vec![0u8; 4096];
        loop {
            let (n, src) = match socket.recv_from(&mut buf).await {
                Ok(v) => v,
                Err(e) => {
                    tracing::debug!(error=%e, "lan discovery recv failed");
                    continue;
                }
            };
            let Ok(beacon) = serde_json::from_slice::<LanBeacon>(&buf[..n]) else {
                continue;
            };
            let now = now_ms();
            if now.abs_diff(beacon.timestamp_ms) > BEACON_MAX_SKEW_MS
                || beacon.period != beacon_period(beacon.timestamp_ms)
                || beacon.tcp_port == 0
            {
                continue;
            }
            let mut matched = None;
            for contact in contacts() {
                let Ok(expected) = contact_token(&contact.shared_secret, beacon.period) else {
                    continue;
                };
                if beacon.token_b64 != B64.encode(expected) {
                    continue;
                }
                let Some(verifying_key) = contact.verifying_key() else {
                    continue;
                };
                if beacon.is_valid_now(&verifying_key) {
                    matched = Some(contact.ed25519_b64);
                    break;
                }
            }
            let Some(contact_id) = matched else { continue };
            let addr = SocketAddr::new(src.ip(), beacon.tcp_port);
            PEERS.note(&contact_id, addr, now);
            tracing::info!("lan: discovered an authenticated contact token");
        }
    });

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::SigningKey;
    use rand::rngs::OsRng;

    #[test]
    fn beacon_contains_only_contact_token_and_rejects_tampering() {
        let key = SigningKey::generate(&mut OsRng);
        let shared = [7u8; 32];
        let period = beacon_period(now_ms());
        let token = contact_token(&shared, period).unwrap();
        let beacon = LanBeacon::create(&key, token, 7777, period);
        assert!(
            beacon.verify(&key.verifying_key()),
            "a freshly signed beacon must verify"
        );
        assert!(beacon.is_valid_now(&key.verifying_key()), "and be fresh");
        assert_eq!(beacon.token_b64, B64.encode(token));
        let json = serde_json::to_string(&beacon).unwrap();
        assert!(
            !json.contains(&B64.encode(key.verifying_key().to_bytes())),
            "the beacon must not reveal the Ed25519 identity"
        );

        // Tampered port breaks the signature.
        let mut tampered = beacon.clone();
        tampered.tcp_port = 9999;
        assert!(
            !tampered.verify(&key.verifying_key()),
            "tampered port must fail verification"
        );

        // A signature from a different key must not verify against this identity.
        let other = SigningKey::generate(&mut OsRng);
        let mut forged = beacon.clone();
        forged.sig_b64 = LanBeacon::create(&other, token, 7777, period).sig_b64;
        assert!(
            !forged.verify(&key.verifying_key()),
            "mismatched signature must fail"
        );
    }

    #[test]
    fn contact_token_rotates_and_stale_beacon_is_rejected() {
        let key = SigningKey::generate(&mut OsRng);
        let shared = [11u8; 32];
        let current_period = beacon_period(now_ms());
        let current = contact_token(&shared, current_period).unwrap();
        let next = contact_token(&shared, current_period + 1).unwrap();
        assert_ne!(current, next, "contact tokens must rotate by period");
        assert_ne!(
            current,
            contact_token(&[12u8; 32], current_period).unwrap(),
            "different contacts must get unlinkable tokens"
        );

        let stale_ms = now_ms() - (BEACON_MAX_SKEW_MS + 60_000);
        let stale_period = beacon_period(stale_ms);
        let stale_token = contact_token(&shared, stale_period).unwrap();
        let beacon = LanBeacon::create_at(&key, stale_token, 7777, stale_period, stale_ms);
        assert!(beacon.verify(&key.verifying_key()), "signature is valid");
        assert!(
            !beacon.is_valid_now(&key.verifying_key()),
            "but a stale beacon must be rejected"
        );
    }

    #[test]
    fn discovered_peers_respects_freshness() {
        let peers = DiscoveredPeers::default();
        let addr: SocketAddr = "192.168.1.5:7777".parse().unwrap();
        peers.note("id-a", addr, now_ms());
        assert_eq!(peers.lookup("id-a"), Some(addr));
        assert_eq!(peers.fresh_count(), 1);
        assert_eq!(peers.lookup("id-unknown"), None);

        // A peer last seen long ago is stale.
        peers.note("id-b", addr, now_ms().saturating_sub(PEER_TTL_MS + 10_000));
        assert_eq!(peers.lookup("id-b"), None);
        assert_eq!(peers.fresh_count(), 1);
    }

    /// End-to-end over localhost: a real signed+encrypted ChatMessage wrapped in
    /// BTP-lite must arrive on the shared inbound channel and parse unchanged.
    #[tokio::test]
    async fn lan_carries_a_real_chat_message_over_localhost() {
        use crate::{build_outbound_message, contact_add, init_profile_with_name};

        // Two throwaway identities that are mutual contacts.
        let adir = tempfile::tempdir().unwrap();
        let bdir = tempfile::tempdir().unwrap();
        init_profile_with_name(adir.path(), "alice").unwrap();
        init_profile_with_name(bdir.path(), "bob").unwrap();
        let a_ed = B64.encode(
            crate::load_signing_key(adir.path())
                .unwrap()
                .verifying_key()
                .to_bytes(),
        );
        let a_x = B64.encode(crate::load_x25519_public(adir.path()).unwrap().as_bytes());
        let b_ed = B64.encode(
            crate::load_signing_key(bdir.path())
                .unwrap()
                .verifying_key()
                .to_bytes(),
        );
        let b_x = B64.encode(crate::load_x25519_public(bdir.path()).unwrap().as_bytes());
        let onion = "qg3dpwh42ldnuy2z42ldce5bc4g6pfpew4s3qti6bngp3hwfrtuvmoqd.onion";
        contact_add(adir.path(), "bob", onion, &b_ed, &b_x).unwrap();
        contact_add(bdir.path(), "alice", onion, &a_ed, &a_x).unwrap();

        let msg =
            build_outbound_message(adir.path(), "bob", "msg", "hi over lan", onion, None).unwrap();
        let line = serde_json::to_string(&msg).unwrap();

        // Bob's LAN listener.
        let (tx, mut rx) = tokio::sync::mpsc::channel::<Envelope>(8);
        let (port, _handle) = spawn_listener(bdir.path().to_path_buf(), tx).await.unwrap();

        // Legacy plaintext is not auto-detected or accepted as a downgrade.
        let addr: SocketAddr = (Ipv4Addr::LOCALHOST, port).into();
        let mut legacy = TcpStream::connect(addr).await.unwrap();
        legacy.write_all(line.as_bytes()).await.unwrap();
        legacy.shutdown().await.unwrap();
        assert!(
            tokio::time::timeout(Duration::from_millis(100), rx.recv())
                .await
                .is_err(),
            "plaintext LAN payload must be dropped"
        );

        // Merely accepting and draining a TCP write is not authenticated
        // delivery. Without the keyed carrier ACK the route must fail so the
        // caller can continue to Tor.
        let no_ack_listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let no_ack_addr = no_ack_listener.local_addr().unwrap();
        let no_ack_peer = tokio::spawn(async move {
            let (mut stream, _) = no_ack_listener.accept().await.unwrap();
            let mut ignored = Vec::new();
            stream.read_to_end(&mut ignored).await.unwrap();
        });
        assert!(send_btp(adir.path(), "bob", no_ack_addr, &line)
            .await
            .is_err());
        no_ack_peer.await.unwrap();

        // Alice sends over localhost TCP.
        send_btp(adir.path(), "bob", addr, &line).await.unwrap();

        // Bob receives the exact wire line, and it parses into a ChatMessage.
        let env = tokio::time::timeout(Duration::from_secs(2), rx.recv())
            .await
            .expect("listener did not deliver in time")
            .expect("inbound channel closed");
        assert_eq!(env.transport_hint.as_deref(), Some("lan"));
        let body = std::str::from_utf8(&env.body).unwrap();
        let parsed = crate::handler::parse_inbound_line(body).unwrap().unwrap();
        assert_eq!(parsed.from, a_ed, "carried message keeps its real sender");
        assert_eq!(parsed.r#type, "msg");
    }
}
