use anyhow::{anyhow, Result};

use super::{Envelope, Transport, TransportCapabilities, TransportStatus};

pub struct TorTransport {
    local_onion: Option<String>,
}

impl TorTransport {
    pub fn new(local_onion: Option<String>) -> Self {
        Self { local_onion }
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
            detail: "tor adapter stub".to_string(),
        }
    }

    fn send(&self, _envelope: &Envelope) -> Result<()> {
        Err(anyhow!("tor transport adapter not wired yet"))
    }

    fn try_recv(&self) -> Result<Option<Envelope>> {
        Ok(None)
    }
}
