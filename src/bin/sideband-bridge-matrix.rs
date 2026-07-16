#![recursion_limit = "512"]

//! Matrix sidecar for Sideband bridge accounts.
//!
//! One process handles one configured provider account. It owns Matrix login,
//! room sync, mautrix bot interaction, and the connector JSON-lines protocol.

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use anyhow::{Context, Result, anyhow};
use base64::{Engine, engine::general_purpose::STANDARD as B64};
use matrix_sdk::{
    Client, Room,
    authentication::matrix::MatrixSession,
    config::SyncSettings,
    media::{MediaFormat, MediaRequestParameters},
    ruma::{
        OwnedRoomId, OwnedUserId,
        api::client::room::create_room::v3::Request as CreateRoomRequest,
        assign,
        events::room::message::{
            MessageType, OriginalSyncRoomMessageEvent, RoomMessageEventContent,
        },
    },
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sideband_bridge_provisioning::{InternalSessionRequest, acquire_internal_session};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    sync::{Mutex, mpsc},
};

const PROTOCOL_VERSION: u32 = 2;

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum CoreMessage {
    Hello {
        protocol: u32,
        account_id: String,
        network: String,
        profile: String,
        config: Value,
    },
    Send {
        outbox_id: i64,
        remote_id: String,
        text: String,
    },
    Login {
        input_id: i64,
    },
    LoginInput {
        input_id: i64,
        step_id: String,
        value: String,
    },
    Shutdown,
}

#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ConnectorMessage {
    Ready,
    Status {
        state: String,
    },
    Conversation {
        remote_id: String,
        title: String,
        kind: String,
    },
    Message {
        remote_id: String,
        event_id: String,
        sender: String,
        text: String,
        timestamp_ms: i64,
    },
    SendResult {
        outbox_id: i64,
        ok: bool,
        error: String,
    },
    LoginInputAck {
        input_id: i64,
    },
    LoginPrompt {
        step_id: String,
        kind: String,
        prompt: String,
        qr: String,
        url: String,
        code: String,
    },
    Error {
        message: String,
    },
}

#[derive(Default)]
struct LoginContext {
    active: bool,
    room_id: Option<OwnedRoomId>,
    bot_user_id: Option<OwnedUserId>,
    sequence: u64,
}

fn require_internal_matrix_session(logged_in: bool) -> Result<()> {
    if logged_in {
        Ok(())
    } else {
        Err(anyhow!(
            "Connected Apps service is unavailable; Sideband could not establish its internal session"
        ))
    }
}

struct Connector {
    network: String,
    config: Value,
    client: Client,
    output: mpsc::UnboundedSender<ConnectorMessage>,
    login: Arc<Mutex<LoginContext>>,
}

impl Connector {
    async fn from_hello(
        account_id: String,
        network: String,
        profile: String,
        config: Value,
        output: mpsc::UnboundedSender<ConnectorMessage>,
    ) -> Result<Self> {
        let homeserver = config
            .get("homeserver")
            .and_then(Value::as_str)
            .filter(|s| !s.trim().is_empty())
            .ok_or_else(|| anyhow!("Matrix homeserver is not configured"))?;
        let profile = PathBuf::from(profile);
        let store_path = profile
            .join("bridge-matrix")
            .join(&account_id)
            .join("store");
        create_private_dir(&store_path).await?;
        let client = Client::builder()
            .homeserver_url(homeserver)
            .sqlite_store(store_path, None)
            .build()
            .await
            .context("build Matrix client")?;

        let session_path = session_path(&profile, &account_id);
        if let Ok(raw) = tokio::fs::read(&session_path).await {
            let session: MatrixSession = serde_json::from_slice(&raw)?;
            client
                .restore_session(session)
                .await
                .context("restore Matrix session")?;
        }

        // No stored session yet: establish Sideband's internal Matrix session
        // from the app-owned credentials (register-then-login), so the user is
        // never asked for Matrix credentials. Requires the backend to be
        // reachable; otherwise `start_login` reports the service is unavailable.
        if !client.matrix_auth().logged_in() {
            establish_internal_session(&client, homeserver, &config, &session_path).await?;
        }

        let login = Arc::new(Mutex::new(LoginContext::default()));
        install_event_handler(&client, &network, output.clone(), login.clone());
        if client.matrix_auth().logged_in() {
            start_sync(client.clone(), output.clone());
        }

        Ok(Self {
            network,
            config,
            client,
            output,
            login,
        })
    }

    async fn start_login(&self) -> Result<()> {
        require_internal_matrix_session(self.client.matrix_auth().logged_in())?;
        self.start_provider_login().await
    }

    async fn submit_login(&self, _step_id: &str, value: &str) -> Result<()> {
        let room_id = self
            .login
            .lock()
            .await
            .room_id
            .clone()
            .ok_or_else(|| anyhow!("provider login room is not ready"))?;
        let room = self
            .client
            .get_room(&room_id)
            .ok_or_else(|| anyhow!("login room disappeared"))?;
        room.send(RoomMessageEventContent::text_plain(value))
            .await?;
        Ok(())
    }

    async fn start_provider_login(&self) -> Result<()> {
        let bot_user_id = self.bot_user_id()?;
        let room = self.provider_room().await?;
        {
            let mut login = self.login.lock().await;
            login.active = true;
            login.room_id = Some(room.room_id().to_owned());
            login.bot_user_id = Some(bot_user_id);
            login.sequence = 0;
        }
        let command = match self.network.as_str() {
            "discord" => "login-qr",
            "messenger" => "login",
            "telegram" => "login qr",
            "googlechat" => "login-cookie",
            _ => {
                return Err(anyhow!(
                    "unsupported Matrix bridge network: {}",
                    self.network
                ));
            }
        };
        room.send(RoomMessageEventContent::text_plain(command))
            .await?;
        self.output
            .send(ConnectorMessage::Status {
                state: "login_required".into(),
            })
            .ok();
        Ok(())
    }

    async fn provider_room(&self) -> Result<Room> {
        if let Some(room_id) = self.config.get("bot_room_id").and_then(Value::as_str) {
            let room_id: OwnedRoomId = room_id.parse()?;
            return self
                .client
                .get_room(&room_id)
                .ok_or_else(|| anyhow!("configured bot room not found"));
        }
        let bot = self.bot_user_id()?;
        let request = assign!(CreateRoomRequest::new(), {
            invite: vec![bot],
            is_direct: true,
        });
        self.client
            .create_room(request)
            .await
            .context("create mautrix bot room")
    }

    fn bot_user_id(&self) -> Result<OwnedUserId> {
        if let Some(user) = self.config.get("bot_user_id").and_then(Value::as_str) {
            return user.parse().map_err(Into::into);
        }
        let own = self
            .client
            .user_id()
            .ok_or_else(|| anyhow!("Matrix client is not logged in"))?;
        let localpart = match self.network.as_str() {
            "telegram" => "telegrambot",
            "discord" => "discordbot",
            "messenger" => "metabot",
            "googlechat" => "googlechatbot",
            _ => return Err(anyhow!("unsupported network")),
        };
        format!("@{localpart}:{}", own.server_name())
            .parse()
            .map_err(Into::into)
    }

    async fn send_message(&self, outbox_id: i64, remote_id: &str, text: &str) {
        let result = async {
            let room_id: OwnedRoomId = remote_id.parse()?;
            let room = self
                .client
                .get_room(&room_id)
                .ok_or_else(|| anyhow!("Matrix room not found"))?;
            room.send(RoomMessageEventContent::text_plain(text)).await?;
            Ok::<_, anyhow::Error>(())
        }
        .await;
        self.output
            .send(ConnectorMessage::SendResult {
                outbox_id,
                ok: result.is_ok(),
                error: result.err().map(|e| e.to_string()).unwrap_or_default(),
            })
            .ok();
    }

    fn prompt(&self, step_id: &str, kind: &str, prompt: &str, qr: &str, url: &str, code: &str) {
        self.output
            .send(ConnectorMessage::LoginPrompt {
                step_id: step_id.into(),
                kind: kind.into(),
                prompt: prompt.into(),
                qr: qr.into(),
                url: url.into(),
                code: code.into(),
            })
            .ok();
    }
}

fn session_path(profile: &Path, account_id: &str) -> PathBuf {
    profile
        .join("bridge-matrix")
        .join(account_id)
        .join("session.json")
}

async fn create_private_dir(path: &Path) -> Result<()> {
    tokio::fs::create_dir_all(path).await?;
    #[cfg(unix)]
    tokio::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).await?;
    Ok(())
}

/// Establish Sideband's internal Matrix session from app-owned credentials.
///
/// Uses the isolated provisioning crate (register-then-login over plain
/// Matrix/Synapse HTTP), restores the resulting session into the matrix-sdk
/// client, and persists it `0600`. The user is never asked for anything: the
/// account localpart, password, and (optional) registration shared secret all
/// arrive in the app-injected `config`. A failure here means the backend is
/// unreachable, surfaced to the UI as the standard "service unavailable" error.
async fn establish_internal_session(
    client: &Client,
    homeserver: &str,
    config: &Value,
    session_path: &Path,
) -> Result<()> {
    let password = config
        .get("internal_password")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| anyhow!("internal account credentials missing"))?;
    let localpart = config
        .get("internal_localpart")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .unwrap_or("sideband");
    let shared_secret = config
        .get("registration_shared_secret")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_string);

    let request = InternalSessionRequest {
        localpart: localpart.to_string(),
        password: password.to_string(),
        shared_secret,
        device_name: "Sideband Connected Apps".to_string(),
    };
    let creds = acquire_internal_session(&reqwest::Client::new(), homeserver, &request)
        .await
        .map_err(|_| {
            anyhow!(
                "Connected Apps service is unavailable; Sideband could not establish its internal session"
            )
        })?;

    // MatrixSession serializes with flattened meta+tokens, so the on-disk shape
    // is `{ user_id, device_id, access_token }` — the same format restored above.
    let session: MatrixSession = serde_json::from_value(serde_json::json!({
        "user_id": creds.user_id,
        "device_id": creds.device_id,
        "access_token": creds.access_token,
    }))
    .context("build internal Matrix session")?;
    client
        .restore_session(session.clone())
        .await
        .context("restore internal Matrix session")?;

    if let Some(parent) = session_path.parent() {
        create_private_dir(parent).await?;
    }
    let raw = serde_json::to_vec(&session).context("serialize internal Matrix session")?;
    tokio::fs::write(session_path, &raw)
        .await
        .context("persist internal Matrix session")?;
    #[cfg(unix)]
    tokio::fs::set_permissions(session_path, std::fs::Permissions::from_mode(0o600)).await?;
    Ok(())
}

fn report_login_error(connector: &Connector, step_id: &str, result: Result<()>) -> bool {
    match result {
        Ok(()) => true,
        Err(error) => {
            connector.prompt(step_id, "error", &error.to_string(), "", "", "");
            false
        }
    }
}

fn install_event_handler(
    client: &Client,
    network: &str,
    output: mpsc::UnboundedSender<ConnectorMessage>,
    login: Arc<Mutex<LoginContext>>,
) {
    let client_for_handler = client.clone();
    let network = network.to_owned();
    client.add_event_handler(move |event: OriginalSyncRoomMessageEvent, room: Room| {
        let output = output.clone();
        let login = login.clone();
        let client = client_for_handler.clone();
        let network = network.clone();
        async move {
            if client.user_id().is_some_and(|id| id == event.sender) {
                return;
            }
            let (login_room, bot_user_id) = {
                let state = login.lock().await;
                (state.room_id.clone(), state.bot_user_id.clone())
            };
            if login_room.as_ref().is_some_and(|id| id == room.room_id()) {
                if bot_user_id.as_ref().is_some_and(|id| id == &event.sender) {
                    relay_login_event(event, &room, &client, &network, &output, &login).await;
                }
                return;
            }
            let title = room
                .display_name()
                .await
                .map(|n| n.to_string())
                .unwrap_or_else(|_| room.room_id().to_string());
            let remote_id = room.room_id().to_string();
            output
                .send(ConnectorMessage::Conversation {
                    remote_id: remote_id.clone(),
                    title,
                    kind: "dm".into(),
                })
                .ok();
            if let MessageType::Text(text) = event.content.msgtype {
                output
                    .send(ConnectorMessage::Message {
                        remote_id,
                        event_id: event.event_id.to_string(),
                        sender: event.sender.to_string(),
                        text: text.body,
                        timestamp_ms: i64::from(event.origin_server_ts.0),
                    })
                    .ok();
            }
        }
    });
}

async fn relay_login_event(
    event: OriginalSyncRoomMessageEvent,
    _room: &Room,
    client: &Client,
    network: &str,
    output: &mpsc::UnboundedSender<ConnectorMessage>,
    login: &Arc<Mutex<LoginContext>>,
) {
    let (kind, prompt, qr, url, code) = match event.content.msgtype {
        MessageType::Image(image) => {
            let params = MediaRequestParameters {
                source: image.source,
                format: MediaFormat::File,
            };
            match client.media().get_media_content(&params, true).await {
                Ok(bytes) => (
                    "qr",
                    format!("Scan with the {network} app"),
                    format!("data:image/png;base64,{}", B64.encode(bytes)),
                    String::new(),
                    String::new(),
                ),
                Err(e) => (
                    "error",
                    format!("Could not download login QR: {e}"),
                    String::new(),
                    String::new(),
                    String::new(),
                ),
            }
        }
        MessageType::Text(text) => classify_login_text(&text.body),
        _ => return,
    };
    let mut state = login.lock().await;
    state.sequence += 1;
    let step_id = format!("provider-{}", state.sequence);
    if kind == "success" || kind == "error" {
        state.active = false;
    }
    output
        .send(ConnectorMessage::LoginPrompt {
            step_id,
            kind: kind.into(),
            prompt,
            qr,
            url,
            code,
        })
        .ok();
}

fn classify_login_text(text: &str) -> (&'static str, String, String, String, String) {
    let lower = text.to_ascii_lowercase();
    let url = text
        .split_whitespace()
        .find(|w| w.starts_with("http://") || w.starts_with("https://"))
        .map(|w| w.trim_matches(|c: char| ",.)]>".contains(c)).to_owned())
        .unwrap_or_default();
    if lower.contains("logged in")
        || lower.contains("login successful")
        || lower.contains("successfully logged")
    {
        (
            "success",
            text.into(),
            String::new(),
            String::new(),
            String::new(),
        )
    } else if lower.contains("error") || lower.contains("failed") {
        (
            "error",
            text.into(),
            String::new(),
            String::new(),
            String::new(),
        )
    } else if !url.is_empty() {
        ("url", text.into(), String::new(), url, String::new())
    } else if lower.contains("password") || lower.contains("2fa") || lower.contains("two-factor") {
        (
            "password_input",
            text.into(),
            String::new(),
            String::new(),
            String::new(),
        )
    } else if lower.contains("phone")
        || lower.contains("token")
        || lower.contains("code")
        || lower.contains("cookie")
    {
        (
            "text_input",
            text.into(),
            String::new(),
            String::new(),
            String::new(),
        )
    } else {
        (
            "code_display",
            text.into(),
            String::new(),
            String::new(),
            text.trim().into(),
        )
    }
}

fn start_sync(client: Client, output: mpsc::UnboundedSender<ConnectorMessage>) {
    tokio::spawn(async move {
        if let Err(e) = client.sync(SyncSettings::default()).await {
            output
                .send(ConnectorMessage::Error {
                    message: format!("Matrix sync stopped: {e}"),
                })
                .ok();
        }
    });
}

#[tokio::main]
async fn main() -> Result<()> {
    let (output_tx, mut output_rx) = mpsc::unbounded_channel::<ConnectorMessage>();
    tokio::spawn(async move {
        let mut stdout = tokio::io::stdout();
        while let Some(message) = output_rx.recv().await {
            if let Ok(mut line) = serde_json::to_vec(&message) {
                line.push(b'\n');
                if stdout.write_all(&line).await.is_err() || stdout.flush().await.is_err() {
                    break;
                }
            }
        }
    });

    let mut lines = BufReader::new(tokio::io::stdin()).lines();
    let mut connector: Option<Connector> = None;
    while let Some(line) = lines.next_line().await? {
        let message: CoreMessage = match serde_json::from_str(&line) {
            Ok(message) => message,
            Err(e) => {
                output_tx
                    .send(ConnectorMessage::Error {
                        message: format!("invalid core message: {e}"),
                    })
                    .ok();
                continue;
            }
        };
        match message {
            CoreMessage::Hello {
                protocol,
                account_id,
                network,
                profile,
                config,
            } => {
                if protocol != PROTOCOL_VERSION {
                    output_tx
                        .send(ConnectorMessage::Error {
                            message: format!(
                                "unsupported bridge protocol {protocol}; expected {PROTOCOL_VERSION}"
                            ),
                        })
                        .ok();
                    continue;
                }
                match Connector::from_hello(account_id, network, profile, config, output_tx.clone())
                    .await
                {
                    Ok(value) => {
                        output_tx.send(ConnectorMessage::Ready).ok();
                        output_tx
                            .send(ConnectorMessage::Status {
                                state: "login_required".into(),
                            })
                            .ok();
                        connector = Some(value);
                    }
                    Err(e) => {
                        output_tx
                            .send(ConnectorMessage::LoginPrompt {
                                step_id: "matrix-config".into(),
                                kind: "error".into(),
                                prompt: e.to_string(),
                                qr: String::new(),
                                url: String::new(),
                                code: String::new(),
                            })
                            .ok();
                    }
                }
            }
            CoreMessage::Login { input_id } => {
                if let Some(c) = &connector
                    && report_login_error(c, "provider-start", c.start_login().await)
                {
                    output_tx
                        .send(ConnectorMessage::LoginInputAck { input_id })
                        .ok();
                }
            }
            CoreMessage::LoginInput {
                input_id,
                step_id,
                value,
            } => {
                if let Some(c) = &connector
                    && report_login_error(c, &step_id, c.submit_login(&step_id, &value).await)
                {
                    output_tx
                        .send(ConnectorMessage::LoginInputAck { input_id })
                        .ok();
                }
            }
            CoreMessage::Send {
                outbox_id,
                remote_id,
                text,
            } => {
                if let Some(c) = &connector {
                    c.send_message(outbox_id, &remote_id, &text).await
                }
            }
            CoreMessage::Shutdown => break,
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_internal_matrix_session_never_requests_user_credentials() {
        let error = require_internal_matrix_session(false).unwrap_err();
        assert_eq!(
            error.to_string(),
            "Connected Apps service is unavailable; Sideband could not establish its internal session"
        );
    }
}
