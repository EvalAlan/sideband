// Shared types for both binary and lib
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiContact {
    pub name: String,
    pub onion: String,
    pub ed25519_pubkey_b64: String,
    pub x25519_pubkey_b64: Option<String>,
    #[serde(default)]
    pub pending: bool,
    #[serde(default)]
    pub blocked: bool,
    /// True when a Double Ratchet (v3) session has been established with this
    /// contact (i.e. the ratchet state file exists on disk).
    #[serde(default)]
    pub ratchet_active: bool,
    /// Live presence: "online" / "away" while the contact's latest heartbeat is
    /// valid, "offline" once it expires, or "" if they never shared presence.
    #[serde(default)]
    pub presence: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiMessage {
    pub id: i64,
    pub direction: String,
    pub contact: String,
    pub onion: String,
    pub body: String,
    pub timestamp_ms: i64,
    pub status: String,
    pub created_at: String,
    pub group_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiStatus {
    pub profile: String,
    pub display_name: String,
    pub contact_count: usize,
    pub transfer_count: usize,
    pub listener_status: String,
    pub listener_onion: String,
    pub listener_running: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiGroup {
    pub id: String,
    pub title: String,
    pub members: Vec<String>,
}
