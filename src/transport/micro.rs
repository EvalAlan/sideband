//! BTP-micro: a compact, lossless binary wire encoding for [`ChatMessage`].
//!
//! For bandwidth-starved carriers (LoRa above all, but LAN/Bluetooth benefit
//! too) JSON is wasteful: field names, quoting, and base64/hex expansion dominate
//! a short text. BTP-micro drops all of that — varint numbers and
//! length-prefixed fields, no keys — while staying a **pure re-encoding**:
//! [`decode`] reconstructs the exact same [`ChatMessage`], so
//! [`crate::payload_to_sign`] and every signature verify unchanged. No crypto
//! changes here.
//!
//! This first cut keeps every string field verbatim (so round-trips are provably
//! lossless). Decoding the base64/hex fields to raw bytes, and dropping the
//! now-redundant Ed25519 signature on established ratchet sessions, are larger
//! wins tracked as separate, crypto-reviewed follow-ups.

use anyhow::{anyhow, bail, Result};

use crate::ChatMessage;

/// Wire-format version for BTP-micro. Bump on any incompatible change.
const FORMAT_VERSION: u8 = 1;

/// Defensive cap on a single decoded field length (bytes). A frame is at most a
/// few hundred bytes on the wire, but a hostile stream could claim more; this
/// bounds work/allocation regardless of the carrier's own limits.
const MAX_FIELD_LEN: usize = 1 << 20; // 1 MiB

fn put_uvarint(out: &mut Vec<u8>, mut value: u128) {
    loop {
        let mut byte = (value & 0x7f) as u8;
        value >>= 7;
        if value != 0 {
            byte |= 0x80;
        }
        out.push(byte);
        if value == 0 {
            break;
        }
    }
}

fn get_uvarint(buf: &[u8], pos: &mut usize) -> Result<u128> {
    let mut result: u128 = 0;
    let mut shift = 0u32;
    loop {
        let byte = *buf.get(*pos).ok_or_else(|| anyhow!("truncated varint"))?;
        *pos += 1;
        // 128 bits / 7 bits per byte = at most 19 bytes; guard before shifting.
        if shift >= 128 {
            bail!("varint overflow");
        }
        result |= ((byte & 0x7f) as u128) << shift;
        if byte & 0x80 == 0 {
            break;
        }
        shift += 7;
    }
    Ok(result)
}

fn put_str(out: &mut Vec<u8>, s: &str) {
    put_uvarint(out, s.len() as u128);
    out.extend_from_slice(s.as_bytes());
}

fn get_str(buf: &[u8], pos: &mut usize) -> Result<String> {
    let len = get_uvarint(buf, pos)? as usize;
    if len > MAX_FIELD_LEN {
        bail!("field length {len} exceeds cap");
    }
    let end = pos
        .checked_add(len)
        .filter(|end| *end <= buf.len())
        .ok_or_else(|| anyhow!("truncated field"))?;
    let s = std::str::from_utf8(&buf[*pos..end])
        .map_err(|_| anyhow!("field is not valid UTF-8"))?
        .to_owned();
    *pos = end;
    Ok(s)
}

/// Encode a [`ChatMessage`] to its compact BTP-micro representation.
pub(crate) fn encode(msg: &ChatMessage) -> Vec<u8> {
    let mut out = Vec::with_capacity(96);
    out.push(FORMAT_VERSION);
    put_uvarint(&mut out, msg.v as u128);
    put_str(&mut out, &msg.r#type);
    put_str(&mut out, &msg.from);
    put_str(&mut out, &msg.sender_name);
    put_str(&mut out, &msg.sender_onion);
    put_str(&mut out, &msg.sender_x25519_pubkey_b64);
    put_uvarint(&mut out, msg.timestamp_ms);
    put_str(&mut out, &msg.body);
    put_str(&mut out, &msg.sig_b64);
    put_str(&mut out, &msg.enc_body);
    put_str(&mut out, &msg.ratchet_header_b64);
    put_str(&mut out, &msg.ratchet_nonce_hex);
    put_str(&mut out, &msg.ratchet_ct_hex);
    match msg.expires_at_ms {
        Some(expires) => {
            out.push(1);
            put_uvarint(&mut out, expires);
        }
        None => out.push(0),
    }
    out
}

/// Decode a BTP-micro frame back into the exact [`ChatMessage`] that produced it.
pub(crate) fn decode(buf: &[u8]) -> Result<ChatMessage> {
    let mut pos = 0usize;
    let version = *buf.get(pos).ok_or_else(|| anyhow!("empty frame"))?;
    pos += 1;
    if version != FORMAT_VERSION {
        bail!("unsupported BTP-micro version {version}");
    }
    let v = u32::try_from(get_uvarint(buf, &mut pos)?).map_err(|_| anyhow!("version overflow"))?;
    let r#type = get_str(buf, &mut pos)?;
    let from = get_str(buf, &mut pos)?;
    let sender_name = get_str(buf, &mut pos)?;
    let sender_onion = get_str(buf, &mut pos)?;
    let sender_x25519_pubkey_b64 = get_str(buf, &mut pos)?;
    let timestamp_ms = get_uvarint(buf, &mut pos)?;
    let body = get_str(buf, &mut pos)?;
    let sig_b64 = get_str(buf, &mut pos)?;
    let enc_body = get_str(buf, &mut pos)?;
    let ratchet_header_b64 = get_str(buf, &mut pos)?;
    let ratchet_nonce_hex = get_str(buf, &mut pos)?;
    let ratchet_ct_hex = get_str(buf, &mut pos)?;
    let has_expiry = *buf
        .get(pos)
        .ok_or_else(|| anyhow!("truncated expiry flag"))?;
    pos += 1;
    let expires_at_ms = match has_expiry {
        0 => None,
        1 => Some(get_uvarint(buf, &mut pos)?),
        other => bail!("invalid expiry flag {other}"),
    };
    if pos != buf.len() {
        bail!("trailing bytes after BTP-micro frame");
    }
    Ok(ChatMessage {
        v,
        r#type,
        from,
        sender_name,
        sender_onion,
        sender_x25519_pubkey_b64,
        timestamp_ms,
        body,
        sig_b64,
        enc_body,
        ratchet_header_b64,
        ratchet_nonce_hex,
        ratchet_ct_hex,
        expires_at_ms,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(v: u32) -> ChatMessage {
        ChatMessage {
            v,
            r#type: "msg".into(),
            from: "Zm9vYmFyZm9vYmFyZm9vYmFyZm9vYmFyZm9vYmFyMDA=".into(),
            sender_name: "alice".into(),
            sender_onion: "fpmansl7byak6gq7ymzi7j3dvetjoi6i3oh2yt4tv5y5wgdnn2icuhid.onion".into(),
            sender_x25519_pubkey_b64: "K4+eWfSYw8TtmsViirLxsNs7zAWzKQ/YtJtQFVcncUk=".into(),
            timestamp_ms: 1_784_000_000_123,
            body: if v == 1 {
                "hello over lora ✓".into()
            } else {
                String::new()
            },
            sig_b64: "c2lnbmF0dXJlc2lnbmF0dXJlc2lnbmF0dXJlc2lnbmF0dXJlc2lnbmF0dXJlc2lnMDA=".into(),
            enc_body: if v == 2 {
                "abcdef0123456789".into()
            } else {
                String::new()
            },
            ratchet_header_b64: if v == 3 {
                "aGVhZGVy".into()
            } else {
                String::new()
            },
            ratchet_nonce_hex: if v == 3 {
                "0011223344556677889900aa".into()
            } else {
                String::new()
            },
            ratchet_ct_hex: if v == 3 {
                "deadbeefcafef00d".into()
            } else {
                String::new()
            },
            expires_at_ms: None,
        }
    }

    #[test]
    fn round_trips_all_versions_and_expiry() {
        for v in [1u32, 2, 3] {
            let mut msg = sample(v);
            assert_eq!(decode(&encode(&msg)).unwrap(), msg, "v{v} must round-trip");
            // With an explicit expiry too.
            msg.expires_at_ms = Some(1_784_100_000_000);
            assert_eq!(
                decode(&encode(&msg)).unwrap(),
                msg,
                "v{v} + expiry must round-trip"
            );
        }
    }

    #[test]
    fn is_smaller_than_json() {
        for v in [1u32, 2, 3] {
            let msg = sample(v);
            let micro = encode(&msg).len();
            let json = serde_json::to_string(&msg).unwrap().len();
            assert!(
                micro < json,
                "v{v}: BTP-micro ({micro}B) should beat JSON ({json}B)"
            );
        }
    }

    #[test]
    fn signature_still_verifies_after_round_trip() {
        // A signed message must verify against its decoded form — proving the
        // re-encoding preserves exactly the bytes payload_to_sign covers.
        let key = ed25519_dalek::SigningKey::generate(&mut rand::rngs::OsRng);
        let mut msg = sample(1);
        msg.from = base64::Engine::encode(
            &base64::engine::general_purpose::STANDARD,
            key.verifying_key().to_bytes(),
        );
        crate::sign_message(&key, &mut msg).unwrap();
        assert!(crate::verify_message_with_sender_metadata(&msg).unwrap());

        let decoded = decode(&encode(&msg)).unwrap();
        assert_eq!(decoded, msg);
        assert!(
            crate::verify_message_with_sender_metadata(&decoded).unwrap(),
            "signature must verify after a BTP-micro round-trip"
        );
    }

    #[test]
    fn rejects_truncated_and_trailing_input() {
        let msg = sample(3);
        let framed = encode(&msg);
        assert!(decode(&framed[..framed.len() - 1]).is_err(), "truncated");
        let mut extended = framed.clone();
        extended.push(0);
        assert!(decode(&extended).is_err(), "trailing bytes");
        assert!(decode(&[]).is_err(), "empty");
        assert!(decode(&[99]).is_err(), "bad version");
    }
}
