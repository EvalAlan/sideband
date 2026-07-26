//! BLE carrier: offline messaging with no OS Bluetooth pairing.
//!
//! Classic RFCOMM can only reach a peer whose address we already know, and on
//! Android 12+ it requires system-level bonding. BLE removes both limits: a
//! device advertises, a scanner recognises it, and they connect — no pairing,
//! no prior address exchange, no internet.
//!
//! The problem BLE creates is *recognition*: a scanner sees anonymous
//! advertisements and must decide "is this one of my contacts?" without leaking
//! a stable identifier to everyone else in the room. We solve that with a
//! **rotating advertisement id** derived from the account key and the current
//! epoch: contacts (who already hold the pubkey) can recompute and match it,
//! while a third party sees an opaque value that changes every epoch.
//!
//! See `docs/plans/2026-07-24-ble-transport.md`.

#![allow(dead_code)]

use sha2::Digest;

/// How often the advertisement id rotates. Short enough that a passive observer
/// can't follow a device around for long; long enough that a scanner only has
/// to check a couple of epochs.
pub(crate) const BLE_ADV_EPOCH_SECS: u64 = 900; // 15 minutes

/// Bytes of the hash we actually advertise. BLE service data is tight (~20
/// usable bytes), and 8 bytes is ample to distinguish a contact list while
/// leaving room for framing.
pub(crate) const BLE_ADV_ID_BYTES: usize = 8;

/// Domain separator, so an advertisement id can never collide with another
/// hash in the protocol.
const BLE_ADV_DOMAIN: &[u8] = b"sideband-ble-adv-v1";

/// The epoch an absolute time falls in.
pub(crate) fn ble_current_epoch(now_ms: u128) -> u64 {
    (now_ms / 1000 / BLE_ADV_EPOCH_SECS as u128) as u64
}

/// The advertisement id a device with `account_pubkey_b64` broadcasts in `epoch`.
pub(crate) fn ble_adv_id(account_pubkey_b64: &str, epoch: u64) -> String {
    let mut hasher = sha2::Sha256::new();
    hasher.update(BLE_ADV_DOMAIN);
    hasher.update(account_pubkey_b64.as_bytes());
    hasher.update(epoch.to_be_bytes());
    hex::encode(&hasher.finalize()[..BLE_ADV_ID_BYTES])
}

/// Match an observed advertisement id against known account keys, tolerating a
/// one-epoch skew in either direction (clock drift, or an advert minted just
/// before a rotation boundary). Returns the matching key.
pub(crate) fn match_ble_adv_id<'a, I>(candidates: I, adv_id: &str, now_ms: u128) -> Option<String>
where
    I: IntoIterator<Item = &'a str>,
{
    let epoch = ble_current_epoch(now_ms);
    let candidates: Vec<&str> = candidates.into_iter().collect();
    for probe in [epoch, epoch.saturating_sub(1), epoch.saturating_add(1)] {
        for key in &candidates {
            if ble_adv_id(key, probe) == adv_id {
                return Some((*key).to_string());
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    const EPOCH_MS: u128 = (BLE_ADV_EPOCH_SECS as u128) * 1000;

    #[test]
    fn a_contacts_advertisement_is_recognised_and_a_strangers_is_not() {
        let alice = "ALICE_PUBKEY_B64";
        let bob = "BOB_PUBKEY_B64";
        let stranger = "STRANGER_PUBKEY_B64";
        let now = 1_700_000_000_000u128;

        // Alice broadcasts; Bob (who has her key) recognises her.
        let advert = ble_adv_id(alice, ble_current_epoch(now));
        assert_eq!(
            match_ble_adv_id([alice, bob], &advert, now),
            Some(alice.to_string())
        );

        // Someone who only knows Bob and a stranger learns nothing from it.
        assert_eq!(match_ble_adv_id([bob, stranger], &advert, now), None);
    }

    #[test]
    fn advertisement_ids_rotate_so_a_device_cannot_be_tracked() {
        let alice = "ALICE_PUBKEY_B64";
        let now = 1_700_000_000_000u128;
        let later = now + EPOCH_MS * 5;

        let a = ble_adv_id(alice, ble_current_epoch(now));
        let b = ble_adv_id(alice, ble_current_epoch(later));
        assert_ne!(a, b, "the advertised id must change between epochs");

        // A stale advert from 5 epochs ago is no longer attributable.
        assert_eq!(match_ble_adv_id([alice], &a, later), None);
        // The id is a short hex string that fits BLE service data.
        assert_eq!(a.len(), BLE_ADV_ID_BYTES * 2);
    }

    #[test]
    fn matching_tolerates_a_one_epoch_skew() {
        let alice = "ALICE_PUBKEY_B64";
        let now = 1_700_000_000_000u128;
        let epoch = ble_current_epoch(now);

        // An advert minted either side of a rotation boundary still matches, so
        // clock drift between two phones doesn't break discovery.
        for probe in [epoch - 1, epoch + 1] {
            let advert = ble_adv_id(alice, probe);
            assert_eq!(
                match_ble_adv_id([alice], &advert, now),
                Some(alice.to_string()),
                "advert from epoch {probe} should match at epoch {epoch}"
            );
        }
    }
}
