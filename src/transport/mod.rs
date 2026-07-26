#![allow(dead_code)]

use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Envelope {
    pub msg_id: String,
    pub from: String,
    pub to: String,
    pub body: Vec<u8>,
    pub seq: u32,
    pub total: u32,
    pub ttl: u8,
    pub hop_count: u8,
    pub transport_hint: Option<String>,
    pub ack_for: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TransportCapabilities {
    pub max_payload: usize,
    pub supports_chunking: bool,
    pub supports_ack: bool,
    pub supports_streaming: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TransportStatus {
    pub connected: bool,
    pub local_addr: Option<String>,
    pub detail: String,
}

#[async_trait]
pub trait Transport: Send + Sync {
    fn name(&self) -> &'static str;
    fn local_addr(&self) -> Option<String>;
    fn capabilities(&self) -> TransportCapabilities;
    fn status(&self) -> TransportStatus;

    async fn send(&self, envelope: &Envelope) -> Result<()>;

    /// Non-blocking poll. Returns Ok(None) when no frame is available right now.
    async fn try_recv(&self) -> Result<Option<Envelope>>;
}

pub mod ble;
pub mod bluetooth;
pub mod btp;
pub mod lan;
pub mod registry;
pub mod tor;
