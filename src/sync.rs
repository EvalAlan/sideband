//! BSP-lite durable inventory and dedup state.
//!
//! Sync IDs belong to retry rows, not `ChatMessage`: adding them to the signed
//! chat schema would break signature verification by older clients. All BSP
//! payloads are themselves carried inside signed and encrypted typed messages.

use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{bail, Result};
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::ChatMessage;

const MAX_SYNC_IDS: usize = 256;
/// Dedup rows older than this are pruned; BSP only needs recent history to
/// suppress duplicate delivery, so retention is bounded like `seen_messages`.
const SYNC_SEEN_RETENTION_DAYS: i64 = 30;
/// Hard cap on retained dedup rows, pruned oldest-first.
const SYNC_SEEN_MAX_ROWS: i64 = 100_000;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct SyncInventoryPayload {
    pub kind: String,
    pub ids: Vec<String>,
    #[serde(default)]
    pub reply: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct SyncRequestPayload {
    pub kind: String,
    pub ids: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct SyncItemPayload {
    pub kind: String,
    pub id: String,
    pub message: ChatMessage,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct SyncAckPayload {
    pub kind: String,
    pub id: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct QueuedSyncItem {
    pub onion: String,
    pub message: String,
    pub expires_at_ms: i64,
}

fn valid_id(id: &str) -> bool {
    id.len() == 32 && id.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn validate_ids(kind: &str, expected: &str, ids: &[String]) -> Result<()> {
    if kind != expected || ids.len() > MAX_SYNC_IDS || ids.iter().any(|id| !valid_id(id)) {
        bail!("invalid {expected} payload");
    }
    Ok(())
}

pub(crate) fn validate_request(request: &SyncRequestPayload) -> Result<()> {
    validate_ids(&request.kind, "sync_request", &request.ids)
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(i64::MAX as u128) as i64
}

pub(crate) fn pending_inventory(
    profile: &Path,
    contact: &str,
    reply: bool,
) -> Result<SyncInventoryPayload> {
    let conn = crate::init_db(profile)?;
    let mut stmt = conn.prepare(
        "SELECT sync_id FROM retry_queue
         WHERE contact=?1 AND sync_id<>'' AND (expires_at_ms=0 OR expires_at_ms>?2)
         ORDER BY created_at ASC LIMIT ?3",
    )?;
    let ids = stmt
        .query_map(params![contact, now_ms(), MAX_SYNC_IDS as i64], |row| {
            row.get::<_, String>(0)
        })?
        .filter_map(|row| row.ok())
        .filter(|id| valid_id(id))
        .collect();
    Ok(SyncInventoryPayload {
        kind: "sync_inventory".into(),
        ids,
        reply,
    })
}

pub(crate) fn missing_from_inventory(
    profile: &Path,
    peer_ed25519: &str,
    inventory: &SyncInventoryPayload,
) -> Result<SyncRequestPayload> {
    validate_ids(&inventory.kind, "sync_inventory", &inventory.ids)?;
    let conn = crate::init_db(profile)?;
    let mut missing = Vec::new();
    for id in &inventory.ids {
        let seen: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM sync_seen_items WHERE peer_ed25519=?1 AND sync_id=?2)",
            params![peer_ed25519, id],
            |row| row.get(0),
        )?;
        if !seen {
            missing.push(id.clone());
        }
    }
    Ok(SyncRequestPayload {
        kind: "sync_request".into(),
        ids: missing,
    })
}

pub(crate) fn mark_received(profile: &Path, peer_ed25519: &str, id: &str) -> Result<bool> {
    if !valid_id(id) {
        bail!("invalid sync item id");
    }
    let conn = crate::init_db(profile)?;
    let inserted = conn.execute(
        "INSERT OR IGNORE INTO sync_seen_items(peer_ed25519, sync_id) VALUES (?1, ?2)",
        params![peer_ed25519, id],
    )? == 1;
    if inserted {
        // Bound the dedup table by age then by absolute count (oldest first) so a
        // long-lived or chatty contact cannot grow it without limit.
        let _ = conn.execute(
            "DELETE FROM sync_seen_items WHERE seen_at < datetime('now', ?1)",
            params![format!("-{SYNC_SEEN_RETENTION_DAYS} days")],
        );
        let _ = conn.execute(
            "DELETE FROM sync_seen_items WHERE rowid IN (
                 SELECT rowid FROM sync_seen_items
                 ORDER BY seen_at DESC, rowid DESC
                 LIMIT -1 OFFSET ?1
             )",
            params![SYNC_SEEN_MAX_ROWS],
        );
    }
    Ok(inserted)
}

pub(crate) fn unmark_received(profile: &Path, peer_ed25519: &str, id: &str) -> Result<()> {
    let conn = crate::init_db(profile)?;
    conn.execute(
        "DELETE FROM sync_seen_items WHERE peer_ed25519=?1 AND sync_id=?2",
        params![peer_ed25519, id],
    )?;
    Ok(())
}

pub(crate) fn queued_item(
    profile: &Path,
    contact: &str,
    id: &str,
) -> Result<Option<QueuedSyncItem>> {
    if !valid_id(id) {
        bail!("invalid sync item id");
    }
    let conn = crate::init_db(profile)?;
    conn.query_row(
        "SELECT onion, message, expires_at_ms FROM retry_queue
         WHERE contact=?1 AND sync_id=?2 AND (expires_at_ms=0 OR expires_at_ms>?3)",
        params![contact, id, now_ms()],
        |row| {
            Ok(QueuedSyncItem {
                onion: row.get(0)?,
                message: row.get(1)?,
                expires_at_ms: row.get(2)?,
            })
        },
    )
    .optional()
    .map_err(Into::into)
}

pub(crate) fn ack_outbound(profile: &Path, contact: &str, id: &str) -> Result<bool> {
    if !valid_id(id) {
        bail!("invalid sync ack id");
    }
    let mut conn = crate::init_db(profile)?;
    let tx = conn.transaction()?;
    let row_id: Option<i64> = tx
        .query_row(
            "SELECT message_row_id FROM retry_queue WHERE contact=?1 AND sync_id=?2",
            params![contact, id],
            |row| row.get(0),
        )
        .optional()?;
    let Some(row_id) = row_id else {
        return Ok(false);
    };
    tx.execute(
        "DELETE FROM retry_queue WHERE contact=?1 AND sync_id=?2",
        params![contact, id],
    )?;
    tx.commit()?;
    crate::mark_message_sent(profile, row_id)?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{enqueue_retry, init_profile_with_name};

    #[test]
    fn inventory_requests_only_missing_unexpired_ids_and_ack_clears_outbox() {
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        init_profile_with_name(a.path(), "alice").unwrap();
        init_profile_with_name(b.path(), "bob").unwrap();
        enqueue_retry(a.path(), "bob", "bob.onion", "one", "offline", None, 0).unwrap();
        enqueue_retry(a.path(), "bob", "bob.onion", "two", "offline", None, 0).unwrap();

        let inventory = pending_inventory(a.path(), "bob", true).unwrap();
        assert_eq!(inventory.ids.len(), 2);
        let request = missing_from_inventory(b.path(), "alice-ed", &inventory).unwrap();
        assert_eq!(request.ids, inventory.ids);

        assert!(mark_received(b.path(), "alice-ed", &request.ids[0]).unwrap());
        assert!(!mark_received(b.path(), "alice-ed", &request.ids[0]).unwrap());
        let request = missing_from_inventory(b.path(), "alice-ed", &inventory).unwrap();
        assert_eq!(request.ids, vec![inventory.ids[1].clone()]);

        let item = queued_item(a.path(), "bob", &inventory.ids[1])
            .unwrap()
            .unwrap();
        assert_eq!(item.message, "two");
        assert!(ack_outbound(a.path(), "bob", &inventory.ids[1]).unwrap());
        assert!(!ack_outbound(a.path(), "bob", &inventory.ids[1]).unwrap());
        assert_eq!(
            pending_inventory(a.path(), "bob", false).unwrap().ids.len(),
            1
        );
    }
}
