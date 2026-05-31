#![allow(dead_code)]

use std::path::Path;
use std::sync::Arc;

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use tokio::sync::{mpsc, oneshot};

use arti_client::TorClient;
use tor_rtcompat::PreferredRuntime;

use super::{Envelope, Transport, TransportCapabilities, TransportStatus};
use crate::TuiEvent;

pub struct TorTransport {
    local_onion: Option<String>,
    pub client: Arc<TorClient<PreferredRuntime>>,
}

impl TorTransport {
    pub fn new(local_onion: Option<String>, client: Arc<TorClient<PreferredRuntime>>) -> Self {
        Self {
            local_onion,
            client,
        }
    }

    pub fn raw_line_to_envelope(raw_line: &str) -> Envelope {
        Envelope {
            msg_id: format!("tor-in-{}", chrono::Utc::now().timestamp_millis()),
            from: String::new(),
            to: String::new(),
            body: raw_line.as_bytes().to_vec(),
            seq: 0,
            total: 1,
            ttl: 1,
            hop_count: 0,
            transport_hint: Some("tor".to_string()),
            ack_for: None,
        }
    }

    pub fn envelope_body_as_str(envelope: &Envelope) -> Result<&str> {
        std::str::from_utf8(&envelope.body)
            .map_err(|_| anyhow!("tor envelope body must be valid utf-8"))
    }

    pub async fn bootstrap(profile: &Path) -> Result<Arc<TorClient<PreferredRuntime>>> {
        Ok(Arc::new(crate::create_tor_client(profile).await?))
    }

    pub async fn serve(
        &self,
        profile: &Path,
        tui_tx: mpsc::Sender<TuiEvent>,
        quit_rx: oneshot::Receiver<()>,
    ) -> Result<()> {
        crate::serve(profile, tui_tx, quit_rx, self.client.clone()).await
    }

    pub async fn send_message(
        &self,
        profile: &Path,
        onion: &str,
        body: &str,
        _contact_name: &str,
    ) -> Result<()> {
        let env = Envelope {
            msg_id: format!("tor-out-{}", chrono::Utc::now().timestamp_millis()),
            from: profile.to_string_lossy().to_string(),
            to: onion.to_string(),
            body: body.as_bytes().to_vec(),
            seq: 0,
            total: 1,
            ttl: 1,
            hop_count: 0,
            transport_hint: Some("tor".to_string()),
            ack_for: None,
        };
        self.send(&env).await
    }

    pub async fn send_file_offer(
        &self,
        profile: &Path,
        contact_name: &str,
        file_path: &str,
    ) -> Result<()> {
        crate::send_file(profile, contact_name, file_path, None, self.client.clone()).await
    }
}

#[async_trait]
impl Transport for TorTransport {
    fn name(&self) -> &'static str {
        "tor"
    }

    fn local_addr(&self) -> Option<String> {
        self.local_onion.clone()
    }

    fn capabilities(&self) -> TransportCapabilities {
        TransportCapabilities {
            max_payload: 64 * 1024,
            supports_chunking: true,
            supports_ack: true,
            supports_streaming: true,
        }
    }

    fn status(&self) -> TransportStatus {
        TransportStatus {
            connected: true,
            local_addr: self.local_onion.clone(),
            detail: "tor transport".to_string(),
        }
    }

    async fn send(&self, envelope: &Envelope) -> Result<()> {
        let body = std::str::from_utf8(&envelope.body)
            .map_err(|_| anyhow!("tor envelope body must be utf-8 encoded chat payload"))?;
        let profile = Path::new(&envelope.from);
        crate::send(
            profile,
            &envelope.to,
            body,
            &envelope.to,
            None,
            self.client.clone(),
        )
        .await
    }

    async fn try_recv(&self) -> Result<Option<Envelope>> {
        Ok(None)
    }
}
