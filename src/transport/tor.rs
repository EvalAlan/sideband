#![allow(dead_code)]

use std::path::Path;
use std::sync::Arc;

use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use futures::StreamExt;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, oneshot};

use arti_client::TorClient;
use safelog::DisplayRedacted;
use tor_rtcompat::PreferredRuntime;

use super::{Envelope, Transport, TransportCapabilities, TransportStatus};
use crate::handler::parse_inbound_line;
use crate::TuiEvent;

pub struct TorTransport {
    local_onion: Option<String>,
    pub client: Arc<TorClient<PreferredRuntime>>,
    status_tx: Option<mpsc::Sender<TuiEvent>>,
    /// Inbound envelope channel.  The receive half is held by `try_recv`.
    inbound_rx: Arc<tokio::sync::Mutex<mpsc::Receiver<Envelope>>>,
    /// Sender kept alive so the channel stays open while run_inbound_loop runs.
    inbound_tx: mpsc::Sender<Envelope>,
}

impl TorTransport {
    pub fn new(local_onion: Option<String>, client: Arc<TorClient<PreferredRuntime>>) -> Self {
        Self::new_with_status(local_onion, client, None)
    }

    pub fn new_with_status(
        local_onion: Option<String>,
        client: Arc<TorClient<PreferredRuntime>>,
        status_tx: Option<mpsc::Sender<TuiEvent>>,
    ) -> Self {
        let (inbound_tx, inbound_rx) = mpsc::channel(256);
        Self {
            local_onion,
            client,
            status_tx,
            inbound_rx: Arc::new(tokio::sync::Mutex::new(inbound_rx)),
            inbound_tx,
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

    /// Run the inbound IO loop: accept Tor connections, bridge to local TCP,
    /// read lines, and push parsed [`Envelope`]s into the inbound channel.
    ///
    /// This is pure transport IO — no protocol dispatch.  The consumer calls
    /// [`Transport::try_recv`] to pull envelopes and handles them.
    ///
    /// Returns when `quit_rx` fires or the onion service stream ends.
    pub async fn run_inbound_loop(&self, mut quit_rx: oneshot::Receiver<()>) -> Result<()> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let listen_port = listener.local_addr()?.port();

        let tor_client = Arc::clone(&self.client);

        // Create an Arti onion service that forwards to our local listener.
        let nickname = tor_hsservice::HsNickname::new("sideband".into())
            .map_err(|e| anyhow!("invalid nickname: {e}"))?;
        let hs_config = tor_hsservice::config::OnionServiceConfigBuilder::default()
            .nickname(nickname)
            .build()
            .context("build onion service config")?;
        let (onion_svc, onion_request_stream) = tor_client
            .launch_onion_service(hs_config)
            .context("launch onion service")?
            .context("onion service disabled or failed to launch")?;
        let onion_hsid = onion_svc
            .onion_address()
            .context("onion service has no address — key may not be ready")?;
        let onion = onion_hsid.display_unredacted().to_string();
        std::env::set_var("SIDEBAND_REPLY_ONION", &onion);
        if let Some(status_tx) = &self.status_tx {
            let _ = status_tx
                .send(TuiEvent::StatusUpdate(format!("onion={onion}")))
                .await;
        }

        tracing::info!(listen_port, onion=%onion, "inbound io loop ready via Arti onion service");

        // Bridge Tor rendezvous requests to local TCP listener.
        let local_addr = format!("127.0.0.1:{listen_port}");
        tokio::spawn(async move {
            let mut stream_requests = tor_hsservice::handle_rend_requests(onion_request_stream);
            while let Some(req) = stream_requests.next().await {
                let local_addr = local_addr.clone();
                tokio::spawn(async move {
                    let mut onion_stream = match req
                        .accept(tor_cell::relaycell::msg::Connected::new_empty())
                        .await
                    {
                        Ok(stream) => stream,
                        Err(e) => {
                            tracing::error!(error=%e, "failed to accept onion stream");
                            return;
                        }
                    };

                    let mut local_stream = match TcpStream::connect(&local_addr).await {
                        Ok(stream) => stream,
                        Err(e) => {
                            tracing::error!(error=%e, %local_addr, "failed to connect onion stream to local listener");
                            return;
                        }
                    };

                    if let Err(e) =
                        tokio::io::copy_bidirectional(&mut onion_stream, &mut local_stream).await
                    {
                        tracing::error!(error=%e, "onion stream bridge failed");
                    }
                });
            }
        });

        let inbound_tx = self.inbound_tx.clone();

        loop {
            tokio::select! {
                incoming = listener.accept() => {
                    let (stream, peer) = incoming?;
                    let inbound_tx = inbound_tx.clone();
                    tracing::info!(%peer, "incoming connection");
                    tokio::spawn(async move {
                        let mut reader = BufReader::new(stream);
                        let mut line = String::new();
                        match reader.read_line(&mut line).await {
                            Ok(0) => {}
                            Ok(_) => {
                                if let Some(_msg) = parse_inbound_line(&line).unwrap_or(None) {
                                    // Push the raw envelope into the try_recv channel.
                                    let env = Self::raw_line_to_envelope(&line);
                                    let _ = inbound_tx.send(env).await;
                                } else {
                                    tracing::error!(raw=%line, "invalid inbound payload");
                                }
                            }
                            Err(e) => tracing::error!(error=%e, "read error"),
                        }
                    });
                }
                _ = &mut quit_rx => {
                    tracing::info!("inbound io loop received quit signal, shutting down");
                    drop(onion_svc);
                    return Ok(());
                }
            }
        }
    }

    pub async fn serve(
        &self,
        profile: &Path,
        tui_tx: mpsc::Sender<TuiEvent>,
        quit_rx: oneshot::Receiver<()>,
    ) -> Result<()> {
        crate::serve(profile, tui_tx, quit_rx, self.client.clone(), false).await
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

        let contacts = crate::load_contacts(profile)?;
        let contact_name = contacts
            .values()
            .find(|c| c.onion == envelope.to)
            .map(|c| c.name.clone())
            .ok_or_else(|| anyhow!("unknown contact for onion '{}'", envelope.to))?;

        crate::send(
            profile,
            &envelope.to,
            body,
            &contact_name,
            None,
            self.client.clone(),
            false,
        )
        .await
    }

    async fn try_recv(&self) -> Result<Option<Envelope>> {
        // Non-blocking poll of the inbound channel.
        let mut rx = self.inbound_rx.lock().await;
        match rx.try_recv() {
            Ok(env) => Ok(Some(env)),
            Err(mpsc::error::TryRecvError::Empty) => Ok(None),
            Err(mpsc::error::TryRecvError::Disconnected) => {
                // Channel closed — inbound loop has terminated.
                Ok(None)
            }
        }
    }
}
