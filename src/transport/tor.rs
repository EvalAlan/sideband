#![allow(dead_code)]

use std::path::Path;
use std::sync::Arc;

use anyhow::{anyhow, Result};
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
        contact_name: &str,
    ) -> Result<()> {
        crate::send(
            profile,
            onion,
            body,
            contact_name,
            None,
            self.client.clone(),
        )
        .await
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

    fn send(&self, _envelope: &Envelope) -> Result<()> {
        Err(anyhow!(
            "generic envelope send not wired for tor yet; use send_message/send_file_offer"
        ))
    }

    fn try_recv(&self) -> Result<Option<Envelope>> {
        Ok(None)
    }
}
