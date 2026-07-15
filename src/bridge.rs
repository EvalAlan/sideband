//! Beeper-style bridge connectors.
//!
//! A *connector* is a sidecar process that translates a non-Sideband network
//! (Telegram, Discord, a Matrix homeserver aggregating mautrix bridges, …) into
//! a small JSON-lines protocol the Sideband core speaks over the child's
//! stdin/stdout. Keeping connectors out-of-process means heavy / AGPL / network
//! specific code never touches the private core or the Android cdylib, and the
//! whole pipeline is testable with a tiny loopback demo connector.
//!
//! Trust boundary: nothing here is E2E-private. Bridged conversations are always
//! tagged `network != "native"` and surfaced to the user as *not* private. This
//! module only ever writes to the `bridge_*` tables and bridged `messages` rows,
//! never to the cryptographic `contacts` domain.

use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Stdio;

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::mpsc;
use tracing::{debug, info, warn};

/// Bump when the wire protocol changes incompatibly.
pub const PROTOCOL_VERSION: u32 = 1;

/// Messages the core sends *to* a connector (one JSON object per line).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CoreToConnector {
    /// Always the first line: identifies the account + hands over its config.
    Hello {
        protocol: u32,
        account_id: String,
        network: String,
        config: serde_json::Value,
    },
    /// Deliver an outbound message to `remote_id`.
    Send {
        outbox_id: i64,
        remote_id: String,
        text: String,
    },
    /// Ask the connector to (re)start its login/auth flow.
    Login,
    /// Graceful shutdown request.
    Shutdown,
}

/// Messages a connector sends *to* the core (one JSON object per line).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ConnectorToCore {
    /// Handshake accepted.
    Ready,
    /// Connection lifecycle: connecting | connected | error | disconnected.
    Status { state: String },
    /// A conversation (chat/room/DM) appeared or changed.
    Conversation {
        remote_id: String,
        #[serde(default)]
        title: String,
        #[serde(default)]
        avatar: String,
        #[serde(default, rename = "kind")]
        kind: String,
        #[serde(default)]
        last_activity_ms: i64,
    },
    /// An inbound message.
    Message {
        remote_id: String,
        #[serde(default)]
        sender: String,
        text: String,
        #[serde(default)]
        timestamp_ms: i64,
    },
    /// Result of a previously requested `Send`.
    SendResult {
        outbox_id: i64,
        ok: bool,
        #[serde(default)]
        error: String,
    },
    /// A login hint to surface to the user (e.g. a QR/URL/code). Phase-2 UI.
    Login {
        #[serde(default)]
        message: String,
        #[serde(default)]
        url_or_code: String,
    },
    /// A non-fatal error to log.
    Error { message: String },
}

/// An inbound event tagged with the account it came from.
struct InboundEvent {
    account_id: String,
    msg: ConnectorToCore,
}

/// A running connector sidecar.
struct ConnectorHandle {
    network: String,
    child: Child,
    to_child: mpsc::UnboundedSender<CoreToConnector>,
}

impl ConnectorHandle {
    fn send(&self, msg: CoreToConnector) {
        // A closed channel just means the writer task died with the child; the
        // manager will notice via the disconnected status and restart it.
        let _ = self.to_child.send(msg);
    }
}

/// Owns the set of running connectors and pumps their traffic to/from the DB.
///
/// The manager is driven by the serve loop: [`Self::reconcile`] starts/stops
/// connectors to match the enabled accounts, [`Self::pump`] drains inbound
/// events into the DB, and [`Self::dispatch_outbox`] pushes queued outbound
/// messages to the right connector. All three are cheap to call on a timer.
pub struct BridgeManager {
    profile: PathBuf,
    connectors: HashMap<String, ConnectorHandle>,
    inbound_tx: mpsc::UnboundedSender<InboundEvent>,
    inbound_rx: mpsc::UnboundedReceiver<InboundEvent>,
}

impl BridgeManager {
    pub fn new(profile: PathBuf) -> Self {
        let (inbound_tx, inbound_rx) = mpsc::unbounded_channel();
        Self {
            profile,
            connectors: HashMap::new(),
            inbound_tx,
            inbound_rx,
        }
    }

    /// Start connectors for enabled accounts and stop ones that were disabled or
    /// removed. Safe to call repeatedly; only acts on differences.
    pub async fn reconcile(&mut self) {
        let accounts = match crate::list_bridge_accounts(&self.profile) {
            Ok(a) => a,
            Err(e) => {
                warn!(error=%e, "bridge: failed to list accounts");
                return;
            }
        };
        let desired: HashMap<String, crate::BridgeAccountRow> = accounts
            .into_iter()
            .filter(|a| a.enabled)
            .map(|a| (a.id.clone(), a))
            .collect();

        // Stop connectors no longer desired.
        let to_stop: Vec<String> = self
            .connectors
            .keys()
            .filter(|id| !desired.contains_key(*id))
            .cloned()
            .collect();
        for id in to_stop {
            self.stop_account(&id).await;
        }

        // Start connectors that are desired but not running (or that exited).
        for (id, account) in desired {
            let dead = self
                .connectors
                .get_mut(&id)
                .map(|h| matches!(h.child.try_wait(), Ok(Some(_)) | Err(_)))
                .unwrap_or(true);
            if dead {
                if self.connectors.contains_key(&id) {
                    self.stop_account(&id).await;
                }
                if let Err(e) = self.start_account(&account).await {
                    warn!(account=%id, error=%e, "bridge: failed to start connector");
                    let _ = crate::set_bridge_account_status(&self.profile, &id, "error");
                }
            }
        }
    }

    async fn start_account(&mut self, account: &crate::BridgeAccountRow) -> Result<()> {
        let (command, args) = resolve_connector_command(account)?;
        info!(account=%account.id, network=%account.network, %command, "bridge: starting connector");

        let mut child = Command::new(&command)
            .args(&args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| anyhow!("spawn {command}: {e}"))?;

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| anyhow!("connector stdin unavailable"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| anyhow!("connector stdout unavailable"))?;
        let stderr = child.stderr.take();

        // Writer task: serialize CoreToConnector as JSON lines to the child.
        let (to_child, mut to_child_rx) = mpsc::unbounded_channel::<CoreToConnector>();
        tokio::spawn(async move {
            let mut stdin = stdin;
            while let Some(msg) = to_child_rx.recv().await {
                match serde_json::to_string(&msg) {
                    Ok(mut line) => {
                        line.push('\n');
                        if stdin.write_all(line.as_bytes()).await.is_err() {
                            break;
                        }
                        let _ = stdin.flush().await;
                    }
                    Err(e) => debug!(error=%e, "bridge: failed to serialize outbound"),
                }
            }
        });

        // Reader task: parse ConnectorToCore JSON lines, forward to the manager.
        let account_id = account.id.clone();
        let inbound_tx = self.inbound_tx.clone();
        tokio::spawn(async move {
            let mut lines = BufReader::new(stdout).lines();
            loop {
                match lines.next_line().await {
                    Ok(Some(line)) => {
                        let line = line.trim();
                        if line.is_empty() {
                            continue;
                        }
                        match serde_json::from_str::<ConnectorToCore>(line) {
                            Ok(msg) => {
                                if inbound_tx
                                    .send(InboundEvent {
                                        account_id: account_id.clone(),
                                        msg,
                                    })
                                    .is_err()
                                {
                                    break;
                                }
                            }
                            Err(e) => {
                                debug!(account=%account_id, error=%e, line=%line, "bridge: bad connector line")
                            }
                        }
                    }
                    Ok(None) => break, // EOF: connector exited.
                    Err(e) => {
                        debug!(account=%account_id, error=%e, "bridge: connector read error");
                        break;
                    }
                }
            }
            // Report the connector as disconnected so the manager restarts it.
            let _ = inbound_tx.send(InboundEvent {
                account_id: account_id.clone(),
                msg: ConnectorToCore::Status {
                    state: "disconnected".into(),
                },
            });
        });

        // Drain stderr to the log so connector diagnostics are visible.
        if let Some(stderr) = stderr {
            let account_id = account.id.clone();
            tokio::spawn(async move {
                let mut lines = BufReader::new(stderr).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    debug!(account=%account_id, "bridge-connector: {line}");
                }
            });
        }

        let handle = ConnectorHandle {
            network: account.network.clone(),
            child,
            to_child,
        };
        handle.send(CoreToConnector::Hello {
            protocol: PROTOCOL_VERSION,
            account_id: account.id.clone(),
            network: account.network.clone(),
            config: serde_json::from_str(&account.config_json).unwrap_or(serde_json::Value::Null),
        });
        self.connectors.insert(account.id.clone(), handle);
        let _ = crate::set_bridge_account_status(&self.profile, &account.id, "connecting");
        Ok(())
    }

    async fn stop_account(&mut self, id: &str) {
        if let Some(mut handle) = self.connectors.remove(id) {
            handle.send(CoreToConnector::Shutdown);
            // kill_on_drop handles the child if it does not exit promptly.
            let _ = handle.child.start_kill();
            let _ = crate::set_bridge_account_status(&self.profile, id, "disconnected");
            debug!(account=%id, "bridge: stopped connector");
        }
    }

    /// Apply all pending inbound events to the DB. Non-blocking.
    pub fn pump(&mut self) {
        while let Ok(event) = self.inbound_rx.try_recv() {
            self.apply(event);
        }
    }

    fn apply(&self, event: InboundEvent) {
        let profile = &self.profile;
        let account_id = &event.account_id;
        let network = self
            .connectors
            .get(account_id)
            .map(|h| h.network.clone())
            .or_else(|| {
                crate::get_bridge_account(profile, account_id)
                    .ok()
                    .flatten()
                    .map(|a| a.network)
            })
            .unwrap_or_else(|| "bridge".to_string());

        match event.msg {
            ConnectorToCore::Ready => {
                debug!(account=%account_id, "bridge: connector ready");
            }
            ConnectorToCore::Status { state } => {
                let _ = crate::set_bridge_account_status(profile, account_id, &state);
            }
            ConnectorToCore::Conversation {
                remote_id,
                title,
                avatar,
                kind,
                last_activity_ms,
            } => {
                let kind = if kind.is_empty() { "dm".into() } else { kind };
                let title = if title.is_empty() {
                    remote_id.clone()
                } else {
                    title
                };
                let _ = crate::upsert_bridge_conversation(
                    profile,
                    account_id,
                    &network,
                    &remote_id,
                    &title,
                    &avatar,
                    &kind,
                    last_activity_ms,
                );
            }
            ConnectorToCore::Message {
                remote_id,
                sender,
                text,
                timestamp_ms,
            } => {
                let ts = if timestamp_ms > 0 {
                    timestamp_ms
                } else {
                    crate::now_ms_i64().unwrap_or(0)
                };
                // Ensure the conversation exists so the message has a home.
                // Pass an empty title so a real title from a `Conversation`
                // event is never clobbered; reads fall back to remote_id.
                let conv_id = crate::upsert_bridge_conversation(
                    profile, account_id, &network, &remote_id, "", "", "dm", ts,
                )
                .unwrap_or_else(|_| crate::bridge_conversation_id(account_id, &remote_id));
                let sender = if sender.is_empty() { remote_id } else { sender };
                let _ = crate::insert_bridge_message(
                    profile, &conv_id, &network, "in", &sender, &text, ts,
                );
            }
            ConnectorToCore::SendResult {
                outbox_id,
                ok,
                error,
            } => {
                let _ = crate::resolve_bridge_outbox(profile, outbox_id, ok, &error);
            }
            ConnectorToCore::Login {
                message,
                url_or_code,
            } => {
                info!(account=%account_id, %message, %url_or_code, "bridge: login hint");
            }
            ConnectorToCore::Error { message } => {
                warn!(account=%account_id, %message, "bridge: connector error");
            }
        }
    }

    /// Push any pending outbound messages to their connectors. Rows whose
    /// connector is not currently running are left pending for a later tick.
    pub fn dispatch_outbox(&mut self) {
        let pending = match crate::take_pending_bridge_outbox(&self.profile, 50) {
            Ok(p) => p,
            Err(e) => {
                debug!(error=%e, "bridge: failed to read outbox");
                return;
            }
        };
        for row in pending {
            let Some(handle) = self.connectors.get(&row.account_id) else {
                continue; // connector not up yet; retry next tick
            };
            handle.send(CoreToConnector::Send {
                outbox_id: row.id,
                remote_id: row.remote_id,
                text: row.body,
            });
            let _ = crate::mark_bridge_outbox_sent(&self.profile, row.id);
        }
    }

    /// Trigger a connector's login flow if it is running. (Phase 2: wired to a
    /// login control command; demo connectors need no login.)
    #[allow(dead_code)]
    pub fn login(&self, account_id: &str) {
        if let Some(handle) = self.connectors.get(account_id) {
            handle.send(CoreToConnector::Login);
        }
    }

    /// Stop every connector (called on serve shutdown).
    pub async fn shutdown(&mut self) {
        let ids: Vec<String> = self.connectors.keys().cloned().collect();
        for id in ids {
            self.stop_account(&id).await;
        }
    }
}

/// Resolve the command + args to launch a connector for `account`.
///
/// The account's `config_json` may specify `{"command": "...", "args": [...]}`.
/// Otherwise we fall back to a bundled connector named `sideband-bridge-<net>`
/// sitting next to the current executable (how the demo + future Matrix
/// connectors ship).
fn resolve_connector_command(account: &crate::BridgeAccountRow) -> Result<(String, Vec<String>)> {
    if let Ok(cfg) = serde_json::from_str::<serde_json::Value>(&account.config_json) {
        if let Some(cmd) = cfg.get("command").and_then(|v| v.as_str()) {
            let args = cfg
                .get("args")
                .and_then(|v| v.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|v| v.as_str().map(str::to_string))
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            return Ok((cmd.to_string(), args));
        }
    }
    let bin = format!("sideband-bridge-{}", account.network);
    if let Some(path) = sibling_binary(&bin) {
        return Ok((path, Vec::new()));
    }
    // Last resort: rely on PATH.
    Ok((bin, Vec::new()))
}

/// Path to `name` (plus platform exe suffix) next to the current executable.
fn sibling_binary(name: &str) -> Option<String> {
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?;
    let mut candidate: PathBuf = dir.join(name);
    if cfg!(windows) {
        candidate.set_extension("exe");
    }
    if candidate.exists() {
        Some(candidate.to_string_lossy().into_owned())
    } else {
        None
    }
}
