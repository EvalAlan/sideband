//! BTP-lite cryptographic framing for non-Tor byte-stream carriers.
//!
//! The plaintext prefix is an eight-byte masked stream counter, random 16-byte stream
//! salt, and 16-byte contact tag. The counter permits a bounded sliding replay
//! window with skipped sends; the salt prevents key/nonce reuse after profile
//! rollback; the tag attributes the stream without exposing an identity or
//! recognizable header. The inner `ChatMessage` remains independently signed
//! and end-to-end encrypted.

use anyhow::{anyhow, bail, Result};
use chacha20poly1305::aead::{Aead, Payload};
use chacha20poly1305::{ChaCha20Poly1305, KeyInit, Nonce};
use hkdf::Hkdf;
use rand::{rngs::OsRng, RngCore};
use sha2::Sha256;
use std::time::{SystemTime, UNIX_EPOCH};
use x25519_dalek::{PublicKey, StaticSecret};

pub(crate) const TAG_LEN: usize = 16;
pub(crate) const STREAM_COUNTER_LEN: usize = 8;
pub(crate) const STREAM_SALT_LEN: usize = 16;
pub(crate) const TAG_OFFSET: usize = STREAM_COUNTER_LEN + STREAM_SALT_LEN;
pub(crate) const PREFIX_LEN: usize = TAG_OFFSET + TAG_LEN;
const HEADER_PLAINTEXT_LEN: usize = 20;
pub(crate) const HEADER_CIPHERTEXT_LEN: usize = HEADER_PLAINTEXT_LEN + 16;
pub(crate) const PREFIX_AND_HEADER_LEN: usize = PREFIX_LEN + HEADER_CIPHERTEXT_LEN;
pub(crate) const MAX_STREAM_CONTENT: usize = 4 * 1024 * 1024;
pub(crate) const PERIOD_SECS: u64 = 30;
/// Clock-skew tolerance for stream-period matching: the receiver scans
/// `current_period ± PERIOD_SKEW`, so peers whose clocks differ by up to
/// `PERIOD_SECS * PERIOD_SKEW` seconds (≈2 min) still connect over LAN/BT. Wider
/// than this and unsynced phones silently fall back to Tor with no error.
pub(crate) const PERIOD_SKEW: u64 = 4;
pub(crate) const STREAM_WINDOW: u64 = 64;

const VERSION: u8 = 1;
const FLAG_FINAL: u8 = 1;
const HEADER_AD: &[u8] = b"sideband/btp-lite/v1/header";
const BODY_AD: &[u8] = b"sideband/btp-lite/v1/body";
const ACK_AD: &[u8] = b"sideband/btp-lite/v1/ack";
pub(crate) const ACK_LEN: usize = 16;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ContactCrypto {
    pub root: [u8; 32],
    /// Direction 0 is lower Ed25519 identity to higher; direction 1 is reverse.
    pub send_direction: u8,
}

impl ContactCrypto {
    pub fn receive_direction(&self) -> u8 {
        self.send_direction ^ 1
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct StreamMaterial {
    pub stream: u64,
    pub wire_stream: u64,
    pub salt: [u8; STREAM_SALT_LEN],
    pub tag: [u8; TAG_LEN],
    key: [u8; 32],
}

pub(crate) struct AuthenticatedStream {
    pub payload: Vec<u8>,
    pub acknowledgement: [u8; ACK_LEN],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PaddingPolicy {
    None,
    Bucketed,
    Full,
}

pub(crate) fn current_period() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
        / PERIOD_SECS
}

pub(crate) fn derive_contact_crypto(
    local_secret: &StaticSecret,
    local_ed25519: [u8; 32],
    remote_x25519: &PublicKey,
    remote_ed25519: [u8; 32],
) -> Result<ContactCrypto> {
    let shared = local_secret.diffie_hellman(remote_x25519).to_bytes();
    if shared.iter().all(|byte| *byte == 0) {
        bail!("invalid all-zero BTP shared secret");
    }
    if local_ed25519 == remote_ed25519 {
        bail!("BTP peer must have a distinct Ed25519 identity");
    }

    let local_x25519 = PublicKey::from(local_secret).to_bytes();
    let mut local_endpoint = [0u8; 64];
    local_endpoint[..32].copy_from_slice(&local_ed25519);
    local_endpoint[32..].copy_from_slice(&local_x25519);
    let mut remote_endpoint = [0u8; 64];
    remote_endpoint[..32].copy_from_slice(&remote_ed25519);
    remote_endpoint[32..].copy_from_slice(remote_x25519.as_bytes());

    let (low, high) = if local_endpoint <= remote_endpoint {
        (&local_endpoint, &remote_endpoint)
    } else {
        (&remote_endpoint, &local_endpoint)
    };
    let mut salt = b"sideband/btp-lite/v1/root-salt".to_vec();
    salt.extend_from_slice(low);
    salt.extend_from_slice(high);
    let hk = Hkdf::<Sha256>::new(Some(&salt), &shared);
    let mut root = [0u8; 32];
    hk.expand(b"sideband/btp-lite/v1/transport-root", &mut root)
        .map_err(|_| anyhow!("derive BTP transport root"))?;

    Ok(ContactCrypto {
        root,
        send_direction: u8::from(local_ed25519 > remote_ed25519),
    })
}

fn stream_mask(
    root: &[u8; 32],
    direction: u8,
    period: u64,
    salt: &[u8; STREAM_SALT_LEN],
) -> Result<u64> {
    let hk = Hkdf::<Sha256>::new(Some(b"sideband/btp-lite/v1/stream-salt"), root);
    let mut info = b"sideband/btp-lite/v1/stream-mask".to_vec();
    info.push(direction);
    info.extend_from_slice(&period.to_be_bytes());
    info.extend_from_slice(salt);
    let mut mask = [0u8; 8];
    hk.expand(&info, &mut mask)
        .map_err(|_| anyhow!("derive BTP stream mask"))?;
    Ok(u64::from_be_bytes(mask))
}

pub(crate) fn recover_stream_number(
    root: &[u8; 32],
    direction: u8,
    period: u64,
    wire_stream: u64,
    salt: &[u8; STREAM_SALT_LEN],
) -> Result<u64> {
    if direction > 1 {
        bail!("invalid BTP direction");
    }
    Ok(wire_stream ^ stream_mask(root, direction, period, salt)?)
}

pub(crate) fn derive_stream_material(
    root: &[u8; 32],
    direction: u8,
    period: u64,
    stream: u64,
    salt: [u8; STREAM_SALT_LEN],
) -> Result<StreamMaterial> {
    if direction > 1 {
        bail!("invalid BTP direction");
    }
    let wire_stream = stream ^ stream_mask(root, direction, period, &salt)?;
    let hk = Hkdf::<Sha256>::new(Some(b"sideband/btp-lite/v1/stream-salt"), root);

    let mut tag_info = b"sideband/btp-lite/v1/stream-tag".to_vec();
    tag_info.push(direction);
    tag_info.extend_from_slice(&period.to_be_bytes());
    tag_info.extend_from_slice(&stream.to_be_bytes());
    tag_info.extend_from_slice(&salt);
    let mut tag = [0u8; TAG_LEN];
    hk.expand(&tag_info, &mut tag)
        .map_err(|_| anyhow!("derive BTP stream tag"))?;

    let mut key_info = b"sideband/btp-lite/v1/stream-key".to_vec();
    key_info.push(direction);
    key_info.extend_from_slice(&period.to_be_bytes());
    key_info.extend_from_slice(&stream.to_be_bytes());
    key_info.extend_from_slice(&salt);
    let mut key = [0u8; 32];
    hk.expand(&key_info, &mut key)
        .map_err(|_| anyhow!("derive BTP stream key"))?;
    Ok(StreamMaterial {
        stream,
        wire_stream,
        salt,
        tag,
        key,
    })
}

pub(crate) fn random_stream_salt() -> [u8; STREAM_SALT_LEN] {
    let mut salt = [0u8; STREAM_SALT_LEN];
    OsRng.fill_bytes(&mut salt);
    salt
}

fn padded_len(content_len: usize, policy: PaddingPolicy) -> Result<usize> {
    if content_len > MAX_STREAM_CONTENT {
        bail!("BTP stream exceeds {MAX_STREAM_CONTENT} bytes");
    }
    match policy {
        PaddingPolicy::None => Ok(content_len),
        PaddingPolicy::Full => Ok(MAX_STREAM_CONTENT),
        PaddingPolicy::Bucketed => {
            const BUCKETS: [usize; 7] = [
                1024,
                4096,
                16 * 1024,
                64 * 1024,
                256 * 1024,
                1024 * 1024,
                MAX_STREAM_CONTENT,
            ];
            BUCKETS
                .into_iter()
                .find(|bucket| *bucket >= content_len)
                .ok_or_else(|| anyhow!("no BTP padding bucket for payload"))
        }
    }
}

fn nonce(domain: u8) -> [u8; 12] {
    let mut nonce = [0u8; 12];
    nonce[0] = domain;
    nonce
}

pub(crate) fn acknowledgement(material: &StreamMaterial) -> Result<[u8; ACK_LEN]> {
    let cipher = ChaCha20Poly1305::new((&material.key).into());
    let mut aad = ACK_AD.to_vec();
    aad.extend_from_slice(&material.stream.to_be_bytes());
    aad.extend_from_slice(&material.salt);
    aad.extend_from_slice(&material.tag);
    let encrypted = cipher
        .encrypt(
            Nonce::from_slice(&nonce(2)),
            Payload {
                msg: &[],
                aad: &aad,
            },
        )
        .map_err(|_| anyhow!("derive BTP acknowledgement"))?;
    encrypted
        .try_into()
        .map_err(|_| anyhow!("invalid BTP acknowledgement length"))
}

pub(crate) fn encode_stream(
    material: &StreamMaterial,
    content: &[u8],
    padding: PaddingPolicy,
) -> Result<Vec<u8>> {
    let padded_len = padded_len(content.len(), padding)?;
    let content_len =
        u32::try_from(content.len()).map_err(|_| anyhow!("content length overflow"))?;
    let padded_len_u32 =
        u32::try_from(padded_len).map_err(|_| anyhow!("padded length overflow"))?;

    let mut header = [0u8; HEADER_PLAINTEXT_LEN];
    header[0] = VERSION;
    header[1] = FLAG_FINAL;
    header[4..12].copy_from_slice(&0u64.to_be_bytes());
    header[12..16].copy_from_slice(&content_len.to_be_bytes());
    header[16..20].copy_from_slice(&padded_len_u32.to_be_bytes());

    let cipher = ChaCha20Poly1305::new((&material.key).into());
    let mut header_ad = HEADER_AD.to_vec();
    header_ad.extend_from_slice(&material.stream.to_be_bytes());
    header_ad.extend_from_slice(&material.salt);
    header_ad.extend_from_slice(&material.tag);
    let encrypted_header = cipher
        .encrypt(
            Nonce::from_slice(&nonce(0)),
            Payload {
                msg: &header,
                aad: &header_ad,
            },
        )
        .map_err(|_| anyhow!("encrypt BTP header"))?;

    let mut body = vec![0u8; padded_len];
    body[..content.len()].copy_from_slice(content);
    if padded_len > content.len() {
        OsRng.fill_bytes(&mut body[content.len()..]);
    }
    let mut body_ad = BODY_AD.to_vec();
    body_ad.extend_from_slice(&material.stream.to_be_bytes());
    body_ad.extend_from_slice(&material.salt);
    body_ad.extend_from_slice(&material.tag);
    body_ad.extend_from_slice(&encrypted_header);
    let encrypted_body = cipher
        .encrypt(
            Nonce::from_slice(&nonce(1)),
            Payload {
                msg: &body,
                aad: &body_ad,
            },
        )
        .map_err(|_| anyhow!("encrypt BTP body"))?;

    let mut wire = Vec::with_capacity(PREFIX_LEN + encrypted_header.len() + encrypted_body.len());
    wire.extend_from_slice(&material.wire_stream.to_be_bytes());
    wire.extend_from_slice(&material.salt);
    wire.extend_from_slice(&material.tag);
    wire.extend_from_slice(&encrypted_header);
    wire.extend_from_slice(&encrypted_body);
    Ok(wire)
}

pub(crate) fn decode_stream(material: &StreamMaterial, wire: &[u8]) -> Result<Vec<u8>> {
    if wire.len() < PREFIX_LEN + HEADER_CIPHERTEXT_LEN + 16 {
        bail!("truncated BTP stream");
    }
    let wire_stream = u64::from_be_bytes(wire[..STREAM_COUNTER_LEN].try_into().unwrap());
    if wire_stream != material.wire_stream
        || wire[STREAM_COUNTER_LEN..TAG_OFFSET] != material.salt
        || wire[TAG_OFFSET..PREFIX_LEN] != material.tag
    {
        bail!("unknown BTP stream tag");
    }
    let encrypted_header = &wire[PREFIX_LEN..PREFIX_LEN + HEADER_CIPHERTEXT_LEN];
    let cipher = ChaCha20Poly1305::new((&material.key).into());
    let mut header_ad = HEADER_AD.to_vec();
    header_ad.extend_from_slice(&material.stream.to_be_bytes());
    header_ad.extend_from_slice(&material.salt);
    header_ad.extend_from_slice(&material.tag);
    let header = cipher
        .decrypt(
            Nonce::from_slice(&nonce(0)),
            Payload {
                msg: encrypted_header,
                aad: &header_ad,
            },
        )
        .map_err(|_| anyhow!("authenticate BTP header"))?;
    if header.len() != HEADER_PLAINTEXT_LEN
        || header[0] != VERSION
        || header[1] != FLAG_FINAL
        || header[2] != 0
        || header[3] != 0
    {
        bail!("invalid BTP header");
    }
    let sequence = u64::from_be_bytes(header[4..12].try_into().unwrap());
    if sequence != 0 {
        bail!("unexpected BTP frame sequence");
    }
    let content_len = u32::from_be_bytes(header[12..16].try_into().unwrap()) as usize;
    let padded_len = u32::from_be_bytes(header[16..20].try_into().unwrap()) as usize;
    if content_len > padded_len || padded_len > MAX_STREAM_CONTENT {
        bail!("invalid BTP frame lengths");
    }
    let expected_wire_len = PREFIX_LEN + HEADER_CIPHERTEXT_LEN + padded_len + 16;
    if wire.len() != expected_wire_len {
        bail!("invalid BTP stream length");
    }

    let encrypted_body = &wire[PREFIX_LEN + HEADER_CIPHERTEXT_LEN..];
    let mut body_ad = BODY_AD.to_vec();
    body_ad.extend_from_slice(&material.stream.to_be_bytes());
    body_ad.extend_from_slice(&material.salt);
    body_ad.extend_from_slice(&material.tag);
    body_ad.extend_from_slice(encrypted_header);
    let body = cipher
        .decrypt(
            Nonce::from_slice(&nonce(1)),
            Payload {
                msg: encrypted_body,
                aad: &body_ad,
            },
        )
        .map_err(|_| anyhow!("authenticate BTP body"))?;
    if body.len() != padded_len {
        bail!("invalid BTP body length");
    }
    Ok(body[..content_len].to_vec())
}

/// Authenticate the fixed prefix and encrypted header, then return the exact
/// total frame length so a carrier can read the padded body without waiting
/// for EOF or buffering unauthenticated megabytes.
pub(crate) fn authenticated_wire_len(
    profile: &std::path::Path,
    prefix_and_header: &[u8],
) -> Result<Option<usize>> {
    if prefix_and_header.len() != PREFIX_AND_HEADER_LEN {
        return Ok(None);
    }
    let wire_stream =
        u64::from_be_bytes(prefix_and_header[..STREAM_COUNTER_LEN].try_into().unwrap());
    let salt: [u8; STREAM_SALT_LEN] = prefix_and_header[STREAM_COUNTER_LEN..TAG_OFFSET]
        .try_into()
        .unwrap();
    let matches = crate::btp_inbound_candidates(profile, wire_stream, salt)?
        .into_iter()
        .filter(|candidate| prefix_and_header[TAG_OFFSET..PREFIX_LEN] == candidate.material.tag)
        .collect::<Vec<_>>();
    if matches.len() != 1 {
        return Ok(None);
    }
    let material = &matches[0].material;
    let cipher = ChaCha20Poly1305::new((&material.key).into());
    let mut header_ad = HEADER_AD.to_vec();
    header_ad.extend_from_slice(&material.stream.to_be_bytes());
    header_ad.extend_from_slice(&material.salt);
    header_ad.extend_from_slice(&material.tag);
    let header = match cipher.decrypt(
        Nonce::from_slice(&nonce(0)),
        Payload {
            msg: &prefix_and_header[PREFIX_LEN..],
            aad: &header_ad,
        },
    ) {
        Ok(header) => header,
        Err(_) => return Ok(None),
    };
    if header.len() != HEADER_PLAINTEXT_LEN
        || header[0] != VERSION
        || header[1] != FLAG_FINAL
        || header[2] != 0
        || header[3] != 0
        || u64::from_be_bytes(header[4..12].try_into().unwrap()) != 0
    {
        return Ok(None);
    }
    let content_len = u32::from_be_bytes(header[12..16].try_into().unwrap()) as usize;
    let padded_len = u32::from_be_bytes(header[16..20].try_into().unwrap()) as usize;
    if content_len > padded_len || padded_len > MAX_STREAM_CONTENT {
        return Ok(None);
    }
    Ok(Some(PREFIX_AND_HEADER_LEN + padded_len + 16))
}

/// Authenticate, decrypt, validate, and transactionally claim one inbound BTP
/// stream. Shared by every non-Tor carrier so tag matching and replay behavior
/// cannot drift between LAN and Bluetooth.
pub(crate) fn decode_authenticated_stream(
    profile: &std::path::Path,
    wire: &[u8],
) -> Result<Option<AuthenticatedStream>> {
    if wire.len() < PREFIX_LEN {
        return Ok(None);
    }
    let wire_stream = u64::from_be_bytes(wire[..STREAM_COUNTER_LEN].try_into().unwrap());
    let salt: [u8; STREAM_SALT_LEN] = wire[STREAM_COUNTER_LEN..TAG_OFFSET].try_into().unwrap();
    let matches = crate::btp_inbound_candidates(profile, wire_stream, salt)?
        .into_iter()
        .filter(|candidate| wire[TAG_OFFSET..PREFIX_LEN] == candidate.material.tag)
        .collect::<Vec<_>>();
    if matches.len() != 1 {
        return Ok(None);
    }
    let candidate = &matches[0];
    let payload = match decode_stream(&candidate.material, wire) {
        Ok(payload) => payload,
        Err(_) => return Ok(None),
    };
    let Ok(line) = std::str::from_utf8(&payload) else {
        return Ok(None);
    };
    if crate::handler::parse_inbound_line(line)?.is_none()
        || !crate::btp_mark_inbound_stream(profile, &candidate.peer_ed25519, candidate.stream_no)?
    {
        return Ok(None);
    }
    Ok(Some(AuthenticatedStream {
        payload,
        acknowledgement: acknowledgement(&candidate.material)?,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn peers() -> (StaticSecret, [u8; 32], StaticSecret, [u8; 32]) {
        let alice = StaticSecret::from([1u8; 32]);
        let bob = StaticSecret::from([2u8; 32]);
        (alice, [3u8; 32], bob, [4u8; 32])
    }

    #[test]
    fn peers_agree_on_root_and_reverse_directions() {
        let (alice_secret, alice_ed, bob_secret, bob_ed) = peers();
        let alice = derive_contact_crypto(
            &alice_secret,
            alice_ed,
            &PublicKey::from(&bob_secret),
            bob_ed,
        )
        .unwrap();
        let bob = derive_contact_crypto(
            &bob_secret,
            bob_ed,
            &PublicKey::from(&alice_secret),
            alice_ed,
        )
        .unwrap();
        assert_eq!(alice.root, bob.root);
        assert_ne!(alice.send_direction, bob.send_direction);
    }

    #[test]
    fn tags_are_separated_by_period_direction_stream_and_contact() {
        let root = [7u8; 32];
        let base = derive_stream_material(&root, 0, 10, 1, [0; STREAM_SALT_LEN]).unwrap();
        assert_ne!(
            base.tag,
            derive_stream_material(&root, 0, 11, 1, [0; STREAM_SALT_LEN])
                .unwrap()
                .tag
        );
        assert_ne!(
            base.tag,
            derive_stream_material(&root, 1, 10, 1, [0; STREAM_SALT_LEN])
                .unwrap()
                .tag
        );
        assert_ne!(
            base.tag,
            derive_stream_material(&root, 0, 10, 2, [0; STREAM_SALT_LEN])
                .unwrap()
                .tag
        );
        assert_ne!(
            base.tag,
            derive_stream_material(&[8u8; 32], 0, 10, 1, [0; STREAM_SALT_LEN])
                .unwrap()
                .tag
        );
        assert_ne!(
            base.tag,
            derive_stream_material(&root, 0, 10, 1, [1; STREAM_SALT_LEN])
                .unwrap()
                .tag
        );
    }

    #[test]
    fn encrypted_stream_round_trips_without_plaintext_and_rejects_tampering() {
        let material = derive_stream_material(&[9u8; 32], 0, 22, 3, [1; STREAM_SALT_LEN]).unwrap();
        let plaintext = br#"{\"type\":\"msg\",\"from\":\"alice\",\"body\":\"secret\"}"#;
        let wire = encode_stream(&material, plaintext, PaddingPolicy::Bucketed).unwrap();
        assert_eq!(
            u64::from_be_bytes(wire[..STREAM_COUNTER_LEN].try_into().unwrap()),
            material.wire_stream
        );
        assert_ne!(material.wire_stream, material.stream);
        assert_eq!(
            recover_stream_number(&[9u8; 32], 0, 22, material.wire_stream, &material.salt,)
                .unwrap(),
            material.stream
        );
        assert_eq!(&wire[STREAM_COUNTER_LEN..TAG_OFFSET], &material.salt);
        assert_eq!(&wire[TAG_OFFSET..PREFIX_LEN], &material.tag);
        assert!(!wire
            .windows(plaintext.len())
            .any(|window| window == plaintext));
        assert_eq!(decode_stream(&material, &wire).unwrap(), plaintext);

        let mut tampered = wire;
        let last = tampered.len() - 1;
        tampered[last] ^= 1;
        assert!(decode_stream(&material, &tampered).is_err());
    }

    #[test]
    fn wrong_tag_or_oversized_payload_fails_closed() {
        let material = derive_stream_material(&[10u8; 32], 0, 1, 0, [2; STREAM_SALT_LEN]).unwrap();
        let mut wire = encode_stream(&material, b"hello", PaddingPolicy::None).unwrap();
        wire[STREAM_COUNTER_LEN] ^= 1;
        assert!(decode_stream(&material, &wire).is_err());
        assert!(encode_stream(
            &material,
            &vec![0u8; MAX_STREAM_CONTENT + 1],
            PaddingPolicy::None,
        )
        .is_err());
    }

    #[test]
    fn all_zero_shared_secret_is_rejected() {
        let secret = StaticSecret::from([1u8; 32]);
        let low_order = PublicKey::from([0u8; 32]);
        assert!(derive_contact_crypto(&secret, [2u8; 32], &low_order, [3u8; 32]).is_err());
    }
}
