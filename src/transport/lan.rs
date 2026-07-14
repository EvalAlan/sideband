//! LAN transport — the first non-Tor carrier.
//!
//! Sideband messages are end-to-end signed + encrypted [`crate::ChatMessage`]s.
//! That crypto is independent of how the bytes travel, so this module simply
//! moves the *same* wire lines over a local TCP socket instead of a Tor circuit.
//! Peers on the same network find each other with signed UDP-broadcast beacons.
//!
//! Security note: the LAN adds no trust. A beacon only advertises "identity X is
//! reachable at this address"; it is self-signed by X's Ed25519 key, so a LAN
//! attacker cannot claim a contact's identity. Even if a beacon were spoofed, the
//! worst case is a message that fails to deliver (Tor remains the fallback) — the
//! ciphertext is never readable and the recipient still verifies the signature.
//!
//! This module is intentionally self-contained and not yet wired into `serve()`
//! or the send path; that integration is the next step.

use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, Result};
use base64::{engine::general_purpose::STANDARD as B64, Engine};
use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
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
/// Cap an inbound LAN line the same way the Tor transport caps its own.
const MAX_INBOUND_LINE: u64 = 512 * 1024;
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

/// A signed presence beacon: "identity `ed25519_b64` accepts LAN connections on
/// `tcp_port`". The sender's IP is taken from the UDP packet, not the beacon.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LanBeacon {
    pub ed25519_b64: String,
    pub tcp_port: u16,
    pub timestamp_ms: u128,
    pub sig_b64: String,
}

impl LanBeacon {
    /// Canonical bytes covered by the signature (everything but the signature).
    fn signing_bytes(ed25519_b64: &str, tcp_port: u16, timestamp_ms: u128) -> Vec<u8> {
        format!("sideband-lan-beacon-v1|{ed25519_b64}|{tcp_port}|{timestamp_ms}").into_bytes()
    }

    /// Build a beacon for our identity advertising `tcp_port`.
    pub fn create(key: &SigningKey, tcp_port: u16) -> Self {
        let ed25519_b64 = B64.encode(key.verifying_key().to_bytes());
        let timestamp_ms = now_ms();
        let sig = key.sign(&Self::signing_bytes(&ed25519_b64, tcp_port, timestamp_ms));
        LanBeacon {
            ed25519_b64,
            tcp_port,
            timestamp_ms,
            sig_b64: B64.encode(sig.to_bytes()),
        }
    }

    /// Verify the beacon is self-signed by the advertised identity. Does NOT
    /// check freshness or whether the identity is a known contact.
    pub fn verify(&self) -> bool {
        let Ok(pk_bytes) = B64.decode(&self.ed25519_b64) else {
            return false;
        };
        let Ok(pk_arr) = <[u8; 32]>::try_from(pk_bytes.as_slice()) else {
            return false;
        };
        let Ok(vk) = VerifyingKey::from_bytes(&pk_arr) else {
            return false;
        };
        let Ok(sig_bytes) = B64.decode(&self.sig_b64) else {
            return false;
        };
        let Ok(sig_arr) = <[u8; 64]>::try_from(sig_bytes.as_slice()) else {
            return false;
        };
        let sig = Signature::from_bytes(&sig_arr);
        let msg = Self::signing_bytes(&self.ed25519_b64, self.tcp_port, self.timestamp_ms);
        vk.verify_strict(&msg, &sig).is_ok()
    }

    /// True if the beacon is signed correctly AND its clock is within the skew
    /// window (rejects replays of very old beacons).
    pub fn is_valid_now(&self) -> bool {
        if !self.verify() {
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

// ---------------------------------------------------------------------------
// TCP carrier
// ---------------------------------------------------------------------------

/// Send a single wire line to a LAN TCP address (a trailing newline is ensured).
pub async fn send_line(addr: SocketAddr, line: &str) -> Result<()> {
    let payload = if line.ends_with('\n') {
        line.to_string()
    } else {
        format!("{line}\n")
    };
    let fut = async {
        let mut stream = TcpStream::connect(addr)
            .await
            .map_err(|e| anyhow!("lan connect {addr}: {e}"))?;
        stream
            .write_all(payload.as_bytes())
            .await
            .map_err(|e| anyhow!("lan write: {e}"))?;
        stream
            .flush()
            .await
            .map_err(|e| anyhow!("lan flush: {e}"))?;
        stream
            .shutdown()
            .await
            .map_err(|e| anyhow!("lan shutdown: {e}"))?;
        Ok::<_, anyhow::Error>(())
    };
    tokio::time::timeout(LAN_SEND_TIMEOUT, fut)
        .await
        .map_err(|_| anyhow!("lan send to {addr} timed out"))?
}

/// Read one newline-terminated line, bounded to [`MAX_INBOUND_LINE`] bytes.
async fn read_line_bounded<R>(reader: &mut R) -> Result<Option<String>>
where
    R: tokio::io::AsyncBufRead + Unpin,
{
    let mut buf = Vec::new();
    let n = AsyncReadExt::take(reader, MAX_INBOUND_LINE + 1)
        .read_until(b'\n', &mut buf)
        .await?;
    if n == 0 {
        return Ok(None);
    }
    if buf.len() as u64 > MAX_INBOUND_LINE {
        return Err(anyhow!(
            "lan inbound line exceeded {MAX_INBOUND_LINE} bytes"
        ));
    }
    let line = String::from_utf8(buf).map_err(|_| anyhow!("lan inbound line is not utf-8"))?;
    Ok(Some(line))
}

/// Wrap a received LAN wire line as an [`Envelope`] for the shared dispatch loop.
fn lan_line_to_envelope(raw_line: &str) -> Envelope {
    Envelope {
        msg_id: format!("lan-in-{}", now_ms()),
        from: String::new(),
        to: String::new(),
        body: raw_line.as_bytes().to_vec(),
        seq: 0,
        total: 1,
        ttl: 1,
        hop_count: 0,
        transport_hint: Some("lan".to_string()),
        ack_for: None,
    }
}

/// Bind a TCP listener for inbound LAN messages and spawn an accept loop that
/// parses each line and forwards valid ones into `inbound_tx` — the same channel
/// `serve()` drains, so LAN and Tor share one `handle_inbound` dispatch.
///
/// Returns the bound port and the accept-loop task handle.
pub async fn spawn_listener(
    inbound_tx: tokio::sync::mpsc::Sender<Envelope>,
) -> Result<(u16, tokio::task::JoinHandle<()>)> {
    let listener = TcpListener::bind((Ipv4Addr::UNSPECIFIED, 0)).await?;
    let port = listener.local_addr()?.port();
    let handle = tokio::spawn(async move {
        loop {
            let (stream, peer) = match listener.accept().await {
                Ok(v) => v,
                Err(e) => {
                    tracing::warn!(error=%e, "lan listener accept failed");
                    continue;
                }
            };
            let inbound_tx = inbound_tx.clone();
            tokio::spawn(async move {
                let mut reader = BufReader::new(stream);
                match read_line_bounded(&mut reader).await {
                    Ok(Some(line)) => {
                        if crate::handler::parse_inbound_line(&line)
                            .unwrap_or(None)
                            .is_some()
                        {
                            let _ = inbound_tx.send(lan_line_to_envelope(&line)).await;
                        } else {
                            tracing::warn!(%peer, "invalid lan inbound payload");
                        }
                    }
                    Ok(None) => {}
                    Err(e) => tracing::warn!(%peer, error=%e, "dropping lan inbound connection"),
                }
            });
        }
    });
    Ok((port, handle))
}

// ---------------------------------------------------------------------------
// UDP-broadcast discovery
// ---------------------------------------------------------------------------

/// Start LAN discovery: broadcast our signed beacon periodically and record the
/// LAN address of any *contact* whose beacon we hear. `accept_identity` gates
/// which advertised identities we trust enough to record (e.g. known contacts),
/// and also filters out our own identity.
///
/// Beacons are written into the process-wide [`PEERS`] registry.
pub async fn spawn_discovery(
    key: SigningKey,
    tcp_port: u16,
    accept_identity: impl Fn(&str) -> bool + Send + Sync + 'static,
) -> Result<()> {
    let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, LAN_DISCOVERY_PORT))
        .await
        .map_err(|e| anyhow!("bind lan discovery udp: {e}"))?;
    socket
        .set_broadcast(true)
        .map_err(|e| anyhow!("enable udp broadcast: {e}"))?;
    let socket = std::sync::Arc::new(socket);

    // Broadcaster.
    let bcast_socket = std::sync::Arc::clone(&socket);
    let bcast_key = key.clone();
    tokio::spawn(async move {
        let dst = SocketAddr::new(IpAddr::V4(Ipv4Addr::BROADCAST), LAN_DISCOVERY_PORT);
        loop {
            let beacon = LanBeacon::create(&bcast_key, tcp_port);
            if let Ok(json) = serde_json::to_vec(&beacon) {
                if let Err(e) = bcast_socket.send_to(&json, dst).await {
                    tracing::debug!(error=%e, "lan beacon broadcast failed");
                }
            }
            tokio::time::sleep(BEACON_INTERVAL).await;
        }
    });

    // Listener.
    let our_id = B64.encode(key.verifying_key().to_bytes());
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
            if beacon.ed25519_b64 == our_id || !beacon.is_valid_now() {
                continue;
            }
            if !accept_identity(&beacon.ed25519_b64) {
                continue;
            }
            let addr = SocketAddr::new(src.ip(), beacon.tcp_port);
            PEERS.note(&beacon.ed25519_b64, addr, now_ms());
            tracing::info!(id=%beacon.ed25519_b64, %addr, "lan: discovered contact");
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
    fn beacon_signs_and_verifies_and_rejects_tampering() {
        let key = SigningKey::generate(&mut OsRng);
        let beacon = LanBeacon::create(&key, 7777);
        assert!(beacon.verify(), "a freshly signed beacon must verify");
        assert!(beacon.is_valid_now(), "and be fresh");

        // Tampered port breaks the signature.
        let mut tampered = beacon.clone();
        tampered.tcp_port = 9999;
        assert!(!tampered.verify(), "tampered port must fail verification");

        // A signature from a different key must not verify against this identity.
        let other = SigningKey::generate(&mut OsRng);
        let mut forged = beacon.clone();
        forged.sig_b64 = LanBeacon::create(&other, 7777).sig_b64;
        assert!(!forged.verify(), "mismatched signature must fail");
    }

    #[test]
    fn stale_beacon_is_signed_but_not_valid_now() {
        let key = SigningKey::generate(&mut OsRng);
        let mut beacon = LanBeacon::create(&key, 7777);
        // Backdate well beyond the skew window.
        beacon.timestamp_ms = now_ms() - (BEACON_MAX_SKEW_MS + 60_000);
        // Re-sign so the signature itself is valid for the stale timestamp.
        let sig = key.sign(&LanBeacon::signing_bytes(
            &beacon.ed25519_b64,
            beacon.tcp_port,
            beacon.timestamp_ms,
        ));
        beacon.sig_b64 = B64.encode(sig.to_bytes());
        assert!(beacon.verify(), "signature is valid");
        assert!(
            !beacon.is_valid_now(),
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

    /// End-to-end over localhost: a real signed+encrypted ChatMessage line sent
    /// with [`send_line`] must arrive on the listener's inbound channel and parse
    /// back into a ChatMessage — proving the LAN carrier moves the real wire
    /// format into the shared dispatch pipeline.
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
        let (port, _handle) = spawn_listener(tx).await.unwrap();

        // Alice sends over localhost TCP.
        let addr: SocketAddr = (Ipv4Addr::LOCALHOST, port).into();
        send_line(addr, &line).await.unwrap();

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
