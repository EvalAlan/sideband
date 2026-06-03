use std::io;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Result;
use base64::Engine;
use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    layout::{Constraint, Direction, Layout, Margin, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{
        Block, Borders, List, ListItem, ListState, Paragraph, Scrollbar, ScrollbarOrientation,
        ScrollbarState,
    },
    Frame, Terminal,
};
use tokio::sync::mpsc;

use crate::TuiEvent;

// TorClient is used implicitly through Arc<TorClient<PreferredRuntime>>
// passed from main.rs / created in run_tui.

// ---------------------------------------------------------------------------
// Outbound command from TUI to send subsystem
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub struct SendCommand {
    pub contact: String,
    pub message: String,
}

#[derive(Debug)]
pub struct FileCommand {
    pub contact: String,
    pub file_path: String,
}

// ---------------------------------------------------------------------------
// App state
// ---------------------------------------------------------------------------

struct App {
    contacts: Vec<String>,
    selected_contact: usize,
    messages: Vec<DisplayMessage>,
    input: String,
    status: String,
    tui_rx: mpsc::Receiver<TuiEvent>,
    send_tx: mpsc::Sender<SendCommand>,
    file_tx: mpsc::Sender<FileCommand>,
    profile: std::path::PathBuf,
    profile_name: String,
    onion: String,
    tor_connected: bool,
    messages_sent: usize,
    messages_recv: usize,
    quit_tx: std::sync::Arc<std::sync::Mutex<Option<tokio::sync::oneshot::Sender<()>>>>,
    scroll_offset: usize,
    last_error: Option<String>,
    error_until: Option<Instant>,
}

struct DisplayMessage {
    direction: String,
    contact: String,
    body: String,
    _timestamp_ms: u128,
    status: String,
    pending: bool,
}

impl App {
    fn new(
        profile: std::path::PathBuf,
        tui_rx: mpsc::Receiver<TuiEvent>,
        send_tx: mpsc::Sender<SendCommand>,
        file_tx: mpsc::Sender<FileCommand>,
        contacts: Vec<String>,
        quit_tx: std::sync::Arc<std::sync::Mutex<Option<tokio::sync::oneshot::Sender<()>>>>,
    ) -> Self {
        let profile_name = crate::load_display_name(&profile).unwrap_or_else(|_| {
            profile
                .file_name()
                .and_then(|n| n.to_str())
                .map(|name| name.trim_start_matches('.'))
                .filter(|name| !name.is_empty())
                .unwrap_or("sideband")
                .to_string()
        });
        Self {
            contacts,
            selected_contact: 0,
            messages: Vec::new(),
            input: String::new(),
            status: "Connecting to Tor…".to_string(),
            tui_rx,
            send_tx,
            file_tx,
            profile,
            profile_name,
            onion: String::new(),
            tor_connected: false,
            messages_sent: 0,
            messages_recv: 0,
            quit_tx,
            scroll_offset: 0,
            last_error: None,
            error_until: None,
        }
    }

    fn try_send(&mut self) -> bool {
        let raw = self.input.trim().to_string();
        if raw.is_empty() {
            return false;
        }

        if raw.starts_with('/') {
            self.input.clear();
            let parts: Vec<&str> = raw[1..].split_whitespace().collect();
            match parts.first().copied() {
                Some("quit") => {
                    self.send_quit();
                    return true;
                }
                Some("q") => {
                    self.push_sys("type /quit to exit", "error");
                    return false;
                }
                Some("send") => {
                    if parts.len() < 3 {
                        self.push_sys("usage: /send <contact> <message>", "error");
                        return false;
                    }
                    let contact = parts[1].to_string();
                    let message = parts[2].to_string();
                    self.do_send(&contact, &message);
                    return false;
                }
                Some("file") => {
                    if parts.len() < 3 {
                        self.push_sys("usage: /file <contact> <filepath>", "error");
                        return false;
                    }
                    let contact = parts[1].to_string();
                    let file_path = parts[2..].join(" ");
                    if file_path.trim().is_empty() {
                        self.push_sys("usage: /file <contact> <filepath>", "error");
                        return false;
                    }
                    self.do_send_file(&contact, &file_path);
                    return false;
                }
                Some("history") => {
                    let contact = parts.get(1).copied();
                    self.show_history(contact);
                    return false;
                }
                Some("help") => {
                    self.push_sys(
                        "/send <contact> <msg>  — send message\n/file <contact> <path> — send file\n/transfers [cancel <hash>|resume <hash>] — list/manage transfers\n/history [contact] — show log\n/contacts — list contacts with keys\n/add <name> <onion> <ed25519_pk> <x25519_pk>\n/delete <contact> — remove contact\n/name [display-name] — show or set your name\n/whoami — show identity keys\n/share  — one-liner for sharing\n/onion  — show onion address\n/ratchet <contact> — init double ratchet\n/status  — full status\n/clear  — clear messages\n/quit   — exit",
                        "help",
                    );
                    return false;
                }
                Some("contacts") => {
                    let full = crate::load_contacts(&self.profile).unwrap_or_default();
                    let list = if full.is_empty() {
                        "(no contacts)".into()
                    } else {
                        let mut lines = Vec::new();
                        for (name, c) in &full {
                            let ratchet_active = crate::RatchetState::path(
                                &self.profile,
                                std::path::Path::new(name),
                            )
                            .exists();
                            let crypto = if ratchet_active {
                                "🔒 v3 local"
                            } else if c.x25519_pubkey_b64.is_some() {
                                "v2 static"
                            } else {
                                "v1 signed"
                            };
                            lines.push(format!(
                                "{}  onion={}  pk={}  crypto={}",
                                name, c.onion, c.pubkey_b64, crypto
                            ));
                        }
                        lines.join("\n")
                    };
                    self.push_sys(&format!("contacts:\n{}", list), "info");
                    return false;
                }
                Some("add") => {
                    if parts.len() < 3 {
                        self.push_sys("usage: /add <contact> <onion> <pubkey>", "error");
                        return false;
                    }
                    let add_parts: Vec<&str> = raw[1..].splitn(5, ' ').collect();
                    if add_parts.len() < 5 {
                        self.push_sys(
                            "usage: /add <name> <onion> <ed25519_pubkey_b64> <x25519_pubkey_b64>",
                            "error",
                        );
                        return false;
                    }
                    let name = add_parts[1].to_string();
                    let onion = add_parts[2].to_string();
                    let pubkey = add_parts[3].to_string();
                    let x25519_pubkey = add_parts[4].to_string();
                    match crate::contact_add(&self.profile, &name, &onion, &pubkey, &x25519_pubkey)
                    {
                        Ok(()) => {
                            self.contacts.push(name.clone());
                            self.contacts.sort();
                            self.push_sys(&format!("contact '{}' added", name), "info");
                        }
                        Err(e) => {
                            self.push_sys(&format!("add failed: {}", e), "error");
                        }
                    }
                    return false;
                }
                Some("delete") => {
                    if parts.len() < 2 {
                        self.push_sys("usage: /delete <contact>", "error");
                        return false;
                    }
                    let name = parts[1].trim().to_string();
                    match crate::contact_delete(&self.profile, &name) {
                        Ok(true) => {
                            self.contacts.retain(|c| c != &name);
                            if self.selected_contact >= self.contacts.len()
                                && !self.contacts.is_empty()
                            {
                                self.selected_contact = self.contacts.len() - 1;
                            }
                            self.push_sys(&format!("contact '{}' deleted", name), "info");
                        }
                        Ok(false) => {
                            self.push_sys(&format!("contact '{}' not found", name), "error");
                        }
                        Err(e) => {
                            self.push_sys(&format!("delete failed: {}", e), "error");
                        }
                    }
                    return false;
                }
                Some("name") => {
                    let new_name = raw.strip_prefix("/name").unwrap_or("").trim();
                    if new_name.is_empty() {
                        self.push_sys(&format!("name: {}", self.profile_name), "info");
                    } else {
                        match crate::set_display_name(&self.profile, new_name) {
                            Ok(name) => {
                                self.profile_name = name.clone();
                                self.push_sys(&format!("name set to: {}", name), "info");
                            }
                            Err(e) => self.push_sys(&format!("name failed: {}", e), "error"),
                        }
                    }
                    return false;
                }
                Some("whoami") => {
                    let pk = crate::load_signing_key(&self.profile)
                        .ok()
                        .map(|k| {
                            let vk = k.verifying_key();
                            base64::engine::general_purpose::STANDARD.encode(vk.to_bytes())
                        })
                        .unwrap_or_else(|| "(error)".into());
                    let x25519_pk = crate::load_x25519_public(&self.profile)
                        .ok()
                        .map(|p| base64::engine::general_purpose::STANDARD.encode(p.as_bytes()))
                        .unwrap_or_else(|| "(error)".into());
                    self.push_sys(
                        &format!(
                            "user: {}\nprofile: {}\npubkey(ed25519): {}\npubkey(x25519): {}",
                            self.profile_name,
                            self.profile.display(),
                            pk,
                            x25519_pk,
                        ),
                        "info",
                    );
                    return false;
                }
                Some("share") => {
                    let pk = crate::load_signing_key(&self.profile)
                        .ok()
                        .map(|k| {
                            let vk = k.verifying_key();
                            base64::engine::general_purpose::STANDARD.encode(vk.to_bytes())
                        })
                        .unwrap_or_else(|| "(error)".into());
                    let x25519_pk = crate::load_x25519_public(&self.profile)
                        .ok()
                        .map(|p| base64::engine::general_purpose::STANDARD.encode(p.as_bytes()))
                        .unwrap_or_else(|| "(error)".into());
                    let onion = if self.onion.is_empty() {
                        "(not yet — waiting for Tor)".into()
                    } else {
                        self.onion.clone()
                    };
                    self.push_sys(
                        &format!(
                            "Send this to your contact:\n  /add {} {} {} {}",
                            self.profile_name, onion, pk, x25519_pk,
                        ),
                        "info",
                    );
                    return false;
                }
                Some("onion") => {
                    let onion = if self.onion.is_empty() {
                        "(not yet — waiting for Tor)".into()
                    } else {
                        self.onion.clone()
                    };
                    self.push_sys(&format!("onion: {}", onion), "info");
                    return false;
                }
                Some("clear") => {
                    self.messages.clear();
                    self.scroll_offset = 0;
                    return false;
                }
                Some("ratchet") => {
                    if parts.len() < 2 {
                        self.push_sys("usage: /ratchet <contact>", "error");
                        return false;
                    }
                    let target = parts[1].trim();
                    if !self.contacts.contains(&target.to_string())
                        && !(self.contacts.len() == 1 && self.contacts[0] == "(no contacts)")
                    {
                        self.push_sys(&format!("unknown contact: {}", target), "error");
                        return false;
                    }
                    match crate::init_ratchet_for_contact(&self.profile, target) {
                        Ok(()) => self.push_sys(
                            &format!("ratchet initialized for '{}' — next message will use double ratchet", target),
                            "info",
                        ),
                        Err(e) => self.push_sys(&format!("ratchet init failed: {}", e), "error"),
                    }
                    return false;
                }
                Some("transfers") => {
                    if parts.len() >= 3 && parts[1] == "cancel" {
                        let hash = parts[2].trim();
                        match crate::cancel_outbound_transfer(&self.profile, hash) {
                            Ok(true) => {
                                self.push_sys(&format!("transfer {} cancelled", hash), "info")
                            }
                            Ok(false) => {
                                self.push_sys(&format!("transfer {} not found", hash), "error")
                            }
                            Err(e) => {
                                self.push_sys(&format!("transfer cancel failed: {}", e), "error")
                            }
                        }
                        return false;
                    }
                    if parts.len() >= 3 && parts[1] == "resume" {
                        let hash = parts[2].trim();
                        match crate::outbound_transfer_target(&self.profile, hash) {
                            Ok(Some((contact, file_path))) => {
                                self.push_sys(&format!("resuming transfer {}", hash), "info");
                                self.do_send_file(&contact, &file_path);
                            }
                            Ok(None) => {
                                self.push_sys(&format!("transfer {} not found", hash), "error")
                            }
                            Err(e) => {
                                self.push_sys(&format!("transfer resume failed: {}", e), "error")
                            }
                        }
                        return false;
                    }
                    match crate::list_transfers(&self.profile) {
                        Ok(rows) if rows.is_empty() => self.push_sys("transfers:\n(none)", "info"),
                        Ok(rows) => {
                            self.push_sys(&format!("transfers:\n{}", rows.join("\n")), "info")
                        }
                        Err(e) => self.push_sys(&format!("transfers failed: {}", e), "error"),
                    }
                    return false;
                }
                Some("status") => {
                    let tor = if self.tor_connected {
                        "connected"
                    } else {
                        "connecting…"
                    };
                    let onion = if self.onion.is_empty() {
                        "(not yet)"
                    } else {
                        &self.onion
                    };
                    // Check per-contact crypto state.
                    let full_contacts = crate::load_contacts(&self.profile).unwrap_or_default();
                    let ratchet_info: Vec<String> = self
                        .contacts
                        .iter()
                        .filter(|c| *c != "(no contacts)")
                        .map(|c| {
                            let active =
                                crate::RatchetState::path(&self.profile, std::path::Path::new(c))
                                    .exists();
                            let label = if active {
                                "🔒 v3 local state"
                            } else if full_contacts
                                .get(c)
                                .and_then(|contact| contact.x25519_pubkey_b64.as_ref())
                                .is_some()
                            {
                                "v2 static encrypted"
                            } else {
                                "v1 signed only"
                            };
                            format!("{}: {}", c, label)
                        })
                        .collect();
                    let ratchet_str = if ratchet_info.is_empty() {
                        "none".into()
                    } else {
                        ratchet_info.join(", ")
                    };
                    let info = format!(
                        "user: {}\nTor: {}\nonion: {}\nsent: {} | recv: {}\ncontacts: {}\nratchet: {}",
                        self.profile_name,
                        tor,
                        onion,
                        self.messages_sent,
                        self.messages_recv,
                        self.contacts.len(),
                        ratchet_str,
                    );
                    self.push_sys(&info, "info");
                    return false;
                }
                Some(cmd) => {
                    self.push_sys(&format!("unknown command: /{}  (try /help)", cmd), "error");
                    return false;
                }
                None => return false,
            }
        }

        // Plain text — send to selected contact.
        if self.contacts.is_empty()
            || (self.contacts.len() == 1 && self.contacts[0] == "(no contacts)")
        {
            self.messages.push(DisplayMessage {
                direction: "out".into(),
                contact: "(nobody)".into(),
                body: raw,
                _timestamp_ms: 0,
                status: "failed (no contact)".into(),
                pending: false,
            });
            self.input.clear();
            return false;
        }
        let contact = self.contacts[self.selected_contact].clone();
        self.do_send(&contact, &raw);
        self.input.clear();
        false
    }

    fn push_sys(&mut self, body: &str, status: &str) {
        let lines: Vec<&str> = body.lines().collect();
        for (i, line) in lines.iter().enumerate() {
            let body = if i == 0 {
                format!("* {}", line)
            } else {
                format!("  {}", line)
            };
            self.messages.push(DisplayMessage {
                direction: "sys".into(),
                contact: " ".into(),
                body,
                _timestamp_ms: 0,
                status: status.into(),
                pending: false,
            });
        }
        const MAX_MESSAGES: usize = 500;
        if self.messages.len() > MAX_MESSAGES {
            let excess = self.messages.len() - MAX_MESSAGES;
            self.messages.drain(0..excess);
        }
    }

    fn do_send(&mut self, contact: &str, message: &str) {
        let _ = self.send_tx.try_send(SendCommand {
            contact: contact.to_string(),
            message: message.to_string(),
        });
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        self.messages.push(DisplayMessage {
            direction: "out".into(),
            contact: contact.to_string(),
            body: message.to_string(),
            _timestamp_ms: ts,
            status: "sending".into(),
            pending: true,
        });
    }

    fn do_send_file(&mut self, contact: &str, file_path: &str) {
        let _ = self.file_tx.try_send(FileCommand {
            contact: contact.to_string(),
            file_path: file_path.to_string(),
        });
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        self.messages.push(DisplayMessage {
            direction: "out".into(),
            contact: contact.to_string(),
            body: format!("[sending file: {}]", file_path),
            _timestamp_ms: ts,
            status: "sending".into(),
            pending: true,
        });
    }

    fn show_history(&mut self, contact: Option<&str>) {
        match crate::load_history(&self.profile, contact, 20) {
            Ok(rows) => {
                if rows.is_empty() {
                    self.messages.push(DisplayMessage {
                        direction: "sys".into(),
                        contact: "*".into(),
                        body: "(no messages)".into(),
                        _timestamp_ms: 0,
                        status: "info".into(),
                        pending: false,
                    });
                } else {
                    for r in rows.into_iter().rev() {
                        let status_label = crate::DeliveryStatus::from_i64(r.status)
                            .map(|s| s.label())
                            .unwrap_or("?");
                        self.messages.push(DisplayMessage {
                            direction: r.direction,
                            contact: r.contact,
                            body: r.body,
                            _timestamp_ms: r.timestamp_ms as u128,
                            status: status_label.into(),
                            pending: false,
                        });
                    }
                }
            }
            Err(e) => {
                self.messages.push(DisplayMessage {
                    direction: "sys".into(),
                    contact: "*".into(),
                    body: format!("history error: {}", e),
                    _timestamp_ms: 0,
                    status: "error".into(),
                    pending: false,
                });
            }
        }
    }

    fn send_quit(&self) {
        if let Some(tx) = self.quit_tx.lock().unwrap().take() {
            let _ = tx.send(());
        }
    }

    fn poll_events(&mut self) {
        while let Ok(evt) = self.tui_rx.try_recv() {
            match evt {
                TuiEvent::InboundMessage {
                    contact,
                    body,
                    timestamp_ms,
                    verified,
                } => {
                    let status = if verified { "verified" } else { "UNVERIFIED" };
                    self.messages.push(DisplayMessage {
                        direction: "in".into(),
                        contact,
                        body,
                        _timestamp_ms: timestamp_ms,
                        status: status.into(),
                        pending: false,
                    });
                    self.messages_recv += 1;
                }
                TuiEvent::OutboundMessage {
                    contact,
                    body,
                    timestamp_ms,
                    status,
                } => {
                    let status_label = format!("{:?}", status);
                    if let Some(msg) = self
                        .messages
                        .iter_mut()
                        .rev()
                        .find(|m| m.pending && m.direction == "out" && m.body == body)
                    {
                        msg.status = status_label;
                        msg.pending = false;
                    } else {
                        self.messages.push(DisplayMessage {
                            direction: "out".into(),
                            contact,
                            body,
                            _timestamp_ms: timestamp_ms,
                            status: status_label,
                            pending: false,
                        });
                    }
                    if status == crate::DeliveryStatus::Sent {
                        self.messages_sent += 1;
                    }
                }
                TuiEvent::StatusUpdate(text) => {
                    if let Some(rest) = text.strip_prefix("onion=") {
                        self.onion = rest.to_string();
                        self.tor_connected = true;
                    } else if text == "Tor ready" || text == "Tor connected" {
                        self.tor_connected = true;
                    }
                    // Show send/file errors in footer and append the full text to the
                    // message pane; the footer often truncates exactly the useful part.
                    if text.starts_with("send error:") || text.starts_with("file send error:") {
                        self.last_error = Some(text.clone());
                        self.error_until = Some(Instant::now() + Duration::from_secs(30));
                        self.messages.push(DisplayMessage {
                            direction: "·".to_string(),
                            contact: "system".to_string(),
                            body: text.clone(),
                            _timestamp_ms: 0,
                            status: "⚠".to_string(),
                            pending: false,
                        });
                    }
                    self.status = text;
                }
            }
        }
        // Clear expired error
        if let Some(until) = self.error_until {
            if Instant::now() > until {
                self.last_error = None;
                self.error_until = None;
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

pub async fn run_tui(profile: &Path) -> Result<()> {
    let (tui_tx, tui_rx) = mpsc::channel::<TuiEvent>(64);
    let (send_tx, mut send_rx) = mpsc::channel::<SendCommand>(16);
    let (file_tx, mut file_rx) = mpsc::channel::<FileCommand>(8);

    let contacts = load_contact_names(profile);

    // Bootstrap Arti Tor client concurrently; TUI shows status until ready.
    let (tor_ready_tx, tor_ready_rx) = tokio::sync::oneshot::channel();
    let profile_for_bootstrap = profile.to_path_buf();
    let tui_tx_for_bootstrap = tui_tx.clone();
    tokio::spawn(async move {
        match crate::transport::tor::TorTransport::bootstrap(&profile_for_bootstrap).await {
            Ok(client) => {
                let _ = tui_tx_for_bootstrap
                    .send(TuiEvent::StatusUpdate("Tor ready".to_string()))
                    .await;
                let _ = tor_ready_tx.send(client);
            }
            Err(e) => {
                let _ = tui_tx_for_bootstrap
                    .send(TuiEvent::StatusUpdate(format!("Tor bootstrap failed: {e}")))
                    .await;
                // Drop tor_ready_tx — the .await in main flow will see Err and bail
                return;
            }
        }
    });

    let profile_buf = profile.to_path_buf();
    let tui_tx_for_serve = tui_tx.clone();
    let tui_tx_for_file = tui_tx.clone();
    let (quit_tx, quit_rx) = tokio::sync::oneshot::channel::<()>();
    let quit_tx = std::sync::Arc::new(std::sync::Mutex::new(Some(quit_tx)));
    let quit_tx_for_run = quit_tx.clone();

    // Spawn serve — it will wait for TorClient to be ready internally
    // For now we pass None and serve polls; simpler: wait here for tor then spawn
    // Actually the cleanest: wait for tor_ready, then spawn serve+send loops
    let tor_client = match tor_ready_rx.await {
        Ok(tc) => tc,
        Err(_) => {
            let _ = tui_tx
                .send(TuiEvent::StatusUpdate(
                    "Tor bootstrap cancelled".to_string(),
                ))
                .await;
            return Err(anyhow::anyhow!("Tor bootstrap failed"));
        }
    };
    let _ = tui_tx
        .send(TuiEvent::StatusUpdate("Tor connected".to_string()))
        .await;

    let tor_for_spawn = Arc::clone(&tor_client);
    let tui_tx_clone = tui_tx.clone();
    tokio::spawn(async move {
        let tor = crate::transport::tor::TorTransport::new(None, tor_for_spawn);
        if let Err(e) = tor.serve(&profile_buf, tui_tx_clone, quit_rx).await {
            let _ = tui_tx_for_serve
                .send(TuiEvent::StatusUpdate(format!("serve error: {e}")))
                .await;
        }
    });

    // Send loop
    let profile_for_send = profile.to_path_buf();
    let tui_tx_for_send = tui_tx.clone();
    let tor_for_send = Arc::clone(&tor_client);
    tokio::spawn(async move {
        while let Some(cmd) = send_rx.recv().await {
            let profile = profile_for_send.clone();
            let tui_tx = tui_tx_for_send.clone();
            let tor = Arc::clone(&tor_for_send);
            tokio::spawn(async move {
                let contact = cmd.contact.clone();
                let message = cmd.message.clone();
                let onion = match crate::resolve_to(&profile, &contact) {
                    Ok(o) => o,
                    Err(e) => {
                        let _ = tui_tx
                            .send(TuiEvent::StatusUpdate(format!("resolve error: {e}")))
                            .await;
                        let _ = tui_tx
                            .send(TuiEvent::OutboundMessage {
                                contact,
                                body: message,
                                timestamp_ms: 0,
                                status: crate::DeliveryStatus::Failed,
                            })
                            .await;
                        return;
                    }
                };
                let ts = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_millis())
                    .unwrap_or(0);
                let tor = crate::transport::tor::TorTransport::new(None, tor);
                match tor.send_message(&profile, &onion, &message, &contact).await {
                    Ok(()) => {
                        let _ = tui_tx
                            .send(TuiEvent::OutboundMessage {
                                contact,
                                body: message,
                                timestamp_ms: ts,
                                status: crate::DeliveryStatus::Sent,
                            })
                            .await;
                    }
                    Err(e) => {
                        let _ = tui_tx
                            .send(TuiEvent::StatusUpdate(format!("send error: {e}")))
                            .await;
                        let _ = tui_tx
                            .send(TuiEvent::OutboundMessage {
                                contact,
                                body: message,
                                timestamp_ms: ts,
                                status: crate::DeliveryStatus::Failed,
                            })
                            .await;
                    }
                }
            });
        }
    });

    // File send loop
    let profile_for_file = profile.to_path_buf();
    let tor_for_file = Arc::clone(&tor_client);
    tokio::spawn(async move {
        while let Some(cmd) = file_rx.recv().await {
            let profile = profile_for_file.clone();
            let tui_tx = tui_tx_for_file.clone();
            let tor = Arc::clone(&tor_for_file);
            tokio::spawn(async move {
                let contact = cmd.contact.clone();
                let file_path = cmd.file_path.clone();
                let tor = crate::transport::tor::TorTransport::new(None, tor);
                match tor.send_file_offer(&profile, &contact, &file_path).await {
                    Ok(()) => {
                        let ts = std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .map(|d| d.as_millis())
                            .unwrap_or(0);
                        let _ = tui_tx
                            .send(TuiEvent::OutboundMessage {
                                contact,
                                body: format!("[file sent: {}]", file_path),
                                timestamp_ms: ts,
                                status: crate::DeliveryStatus::Sent,
                            })
                            .await;
                    }
                    Err(e) => {
                        let _ = tui_tx
                            .send(TuiEvent::StatusUpdate(format!("file send error: {e}")))
                            .await;
                        let _ = tui_tx
                            .send(TuiEvent::OutboundMessage {
                                contact,
                                body: format!("[file failed: {}]", file_path),
                                timestamp_ms: 0,
                                status: crate::DeliveryStatus::Failed,
                            })
                            .await;
                    }
                }
            });
        }
    });

    tokio::time::sleep(Duration::from_millis(500)).await;

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = ratatui::backend::CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut app = App::new(
        profile.to_path_buf(),
        tui_rx,
        send_tx,
        file_tx,
        contacts,
        quit_tx_for_run,
    );

    let tick_rate = Duration::from_millis(250);
    let mut last_tick = Instant::now();

    loop {
        terminal.draw(|f| draw(f, &app))?;

        let timeout = tick_rate.saturating_sub(last_tick.elapsed());
        if crossterm::event::poll(timeout)? {
            if let Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Press {
                    match key.code {
                        KeyCode::Enter => {
                            if app.try_send() {
                                break;
                            }
                        }
                        KeyCode::Char(c) => {
                            app.input.push(c);
                        }
                        KeyCode::Backspace => {
                            app.input.pop();
                        }
                        KeyCode::Esc => {
                            app.input.clear();
                        }
                        KeyCode::Tab => {
                            if !app.contacts.is_empty() {
                                app.selected_contact =
                                    (app.selected_contact + 1) % app.contacts.len();
                            }
                        }
                        KeyCode::Up => {
                            if app.selected_contact > 0 {
                                app.selected_contact -= 1;
                            }
                        }
                        KeyCode::Down => {
                            if !app.contacts.is_empty()
                                && app.selected_contact < app.contacts.len() - 1
                            {
                                app.selected_contact += 1;
                            }
                        }
                        KeyCode::PageUp => {
                            app.scroll_offset = app.scroll_offset.saturating_add(5);
                        }
                        KeyCode::PageDown => {
                            app.scroll_offset = app.scroll_offset.saturating_sub(5);
                        }
                        _ => {}
                    }
                }
            }
        }

        if last_tick.elapsed() >= tick_rate {
            app.poll_events();
            last_tick = Instant::now();
        }
    }

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    Ok(())
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn load_contact_names(profile: &Path) -> Vec<String> {
    let p = profile.join("contacts.toml");
    if !p.exists() {
        return vec!["(no contacts)".into()];
    }
    let text = match std::fs::read_to_string(&p) {
        Ok(t) => t,
        Err(_) => return vec!["(error)".into()],
    };
    let map: std::collections::HashMap<String, toml::Value> = match toml::from_str(&text) {
        Ok(m) => m,
        Err(_) => return vec!["(parse error)".into()],
    };
    let mut names: Vec<String> = map.keys().cloned().collect();
    names.sort();
    if names.is_empty() {
        names.push("(no contacts)".into());
    }
    names
}

// ---------------------------------------------------------------------------
// Drawing
// ---------------------------------------------------------------------------

fn draw(f: &mut Frame, app: &App) {
    let area = f.area();

    // Root layout: [header] [main: contacts | messages] [input] [footer]
    let root = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // header
            Constraint::Min(5),    // main
            Constraint::Length(3), // input
            Constraint::Length(1), // footer
        ])
        .split(area);

    // Main: contacts (20%) | messages (80%)
    let main = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(20), Constraint::Percentage(80)])
        .split(root[1]);

    draw_header(f, root[0], app);
    draw_contacts(f, main[0], app);
    draw_messages(f, main[1], app);
    draw_input(f, root[2], app);
    draw_footer(f, root[3], app);
}

fn draw_header(f: &mut Frame, area: Rect, app: &App) {
    let onion_short = if app.onion.len() > 16 {
        format!("{}…", &app.onion[..16])
    } else {
        app.onion.clone()
    };
    let tor_indicator = if app.tor_connected {
        Span::styled("●", Color::Green)
    } else {
        Span::styled("○", Color::Red)
    };
    let text = Line::from(vec![
        Span::styled(
            " Sideband ",
            Style::default()
                .fg(Color::White)
                .bg(Color::Blue)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" "),
        tor_indicator,
        Span::raw(" "),
        Span::styled(onion_short, Style::default().fg(Color::Cyan)),
        Span::raw("  "),
        Span::styled(
            format!("@{}", crate::BUILD_COMMIT),
            Style::default().fg(Color::Magenta),
        ),
        Span::raw("  "),
        Span::styled(
            format!("↑{}", app.messages_sent),
            Style::default().fg(Color::Green),
        ),
        Span::raw(" "),
        Span::styled(
            format!("↓{}", app.messages_recv),
            Style::default().fg(Color::Yellow),
        ),
    ]);
    let header = Paragraph::new(text).style(Style::default().bg(Color::DarkGray));
    f.render_widget(header, area);
}

fn draw_contacts(f: &mut Frame, area: Rect, app: &App) {
    let items: Vec<ListItem> = app
        .contacts
        .iter()
        .enumerate()
        .map(|(i, name)| {
            let ratchet_active =
                crate::RatchetState::path(&app.profile, std::path::Path::new(name)).exists();
            let display = if ratchet_active && name != "(no contacts)" {
                format!("🔒 {}", name)
            } else {
                name.clone()
            };
            let style = if i == app.selected_contact {
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Cyan)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::White)
            };
            ListItem::new(Span::styled(display, style))
        })
        .collect();

    let widget = List::new(items).block(
        Block::default()
            .title("Contacts")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Cyan)),
    );
    f.render_stateful_widget(
        widget,
        area,
        &mut ListState::default().with_selected(Some(app.selected_contact)),
    );
}

fn chat_scroll_position(
    message_count: usize,
    visible_height: usize,
    scroll_offset_from_bottom: usize,
) -> usize {
    let max_scroll = message_count.saturating_sub(visible_height);
    max_scroll.saturating_sub(scroll_offset_from_bottom.min(max_scroll))
}

fn wrap_text(text: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let mut out = Vec::new();

    for raw_line in text.split('\n') {
        let mut current = String::new();
        for word in raw_line.split_whitespace() {
            if word.chars().count() > width {
                if !current.is_empty() {
                    out.push(std::mem::take(&mut current));
                }
                let mut chunk = String::new();
                for ch in word.chars() {
                    chunk.push(ch);
                    if chunk.chars().count() >= width {
                        out.push(std::mem::take(&mut chunk));
                    }
                }
                if !chunk.is_empty() {
                    current = chunk;
                }
            } else if current.is_empty() {
                current.push_str(word);
            } else if current.chars().count() + 1 + word.chars().count() <= width {
                current.push(' ');
                current.push_str(word);
            } else {
                out.push(std::mem::take(&mut current));
                current.push_str(word);
            }
        }
        if current.is_empty() {
            out.push(String::new());
        } else {
            out.push(current);
        }
    }

    out
}

fn draw_messages(f: &mut Frame, area: Rect, app: &App) {
    let selected = if app.contacts.is_empty() || app.contacts[0] == "(no contacts)" {
        None
    } else {
        Some(app.contacts[app.selected_contact].as_str())
    };

    let visible: Vec<&DisplayMessage> = app
        .messages
        .iter()
        .filter(|m| {
            if m.direction != "in" && m.direction != "out" {
                return true; // keep system/status lines visible
            }
            match selected {
                Some(name) => m.contact == name,
                None => true,
            }
        })
        .collect();

    let inner_width = area.width.saturating_sub(2) as usize; // subtract borders
    let items: Vec<ListItem> = visible
        .iter()
        .flat_map(|m| {
            let dir_color = match m.direction.as_str() {
                "in" => Color::Green,
                "out" => Color::Blue,
                _ => Color::Yellow,
            };
            let dir_label = match m.direction.as_str() {
                "in" => "←",
                "out" => "→",
                _ => "·",
            };
            let status_icon = match m.status.as_str() {
                "sending" => Some(("⏳", Color::Yellow)),
                "Failed" | "failed" | "failed (no contact)" => Some(("✗", Color::Red)),
                "UNVERIFIED" => Some(("⚠", Color::Red)),
                _ => None,
            };
            let ts_str = if m._timestamp_ms > 0 {
                let secs = (m._timestamp_ms / 1000) as i64;
                let naive = chrono::DateTime::from_timestamp(secs, 0)
                    .map(|dt| dt.format("%H:%M:%S").to_string())
                    .unwrap_or_else(|| "??:??:?".into());
                format!("{naive} ")
            } else {
                String::new()
            };

            let display_contact = if m.direction == "out" {
                app.profile_name.as_str()
            } else {
                m.contact.as_str()
            };
            let prefix = format!("{ts_str}{dir_label} {:>12} ", display_contact);
            let prefix_width = prefix.chars().count();
            let body_width = inner_width.saturating_sub(prefix_width).max(8);
            let mut chunks = wrap_text(&m.body, body_width);
            if chunks.is_empty() {
                chunks.push(String::new());
            }
            let last = chunks.len().saturating_sub(1);

            chunks
                .into_iter()
                .enumerate()
                .map(|(idx, chunk)| {
                    let mut spans = if idx == 0 {
                        vec![
                            Span::styled(
                                prefix.clone(),
                                Style::default().fg(dir_color).add_modifier(Modifier::BOLD),
                            ),
                            Span::raw(chunk),
                        ]
                    } else {
                        vec![Span::raw(" ".repeat(prefix_width)), Span::raw(chunk)]
                    };
                    if idx == last {
                        if let Some((icon, color)) = status_icon {
                            spans.push(Span::styled(
                                format!(" {}", icon),
                                Style::default().fg(color),
                            ));
                        }
                    }
                    ListItem::new(Line::from(spans))
                })
                .collect::<Vec<_>>()
        })
        .collect();

    let msg_len = items.len();
    // Chat UIs should follow the bottom by default. Treat scroll_offset as
    // "lines back from newest": 0 = bottom, PgUp = older, PgDn = newer.
    let visible_height = area.height.saturating_sub(2) as usize; // subtract borders
    let scroll = chat_scroll_position(msg_len, visible_height, app.scroll_offset);

    let widget = List::new(items).block(
        Block::default()
            .title("Messages")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::White)),
    );

    let mut state = ListState::default().with_offset(scroll);
    f.render_stateful_widget(widget, area, &mut state);

    // Scrollbar
    if msg_len > visible_height {
        let max_scroll = msg_len.saturating_sub(visible_height);
        let mut scrollbar_state = ScrollbarState::new(max_scroll).position(scroll);
        let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
            .thumb_style(Style::default().fg(Color::Cyan));
        let scrollbar_area = area.inner(Margin {
            vertical: 1,
            horizontal: 0,
        });
        f.render_stateful_widget(scrollbar, scrollbar_area, &mut scrollbar_state);
    }
}

#[cfg(test)]
mod tests {
    use super::{chat_scroll_position, wrap_text};

    #[test]
    fn chat_scroll_defaults_to_bottom() {
        assert_eq!(chat_scroll_position(100, 20, 0), 80);
    }

    #[test]
    fn chat_scroll_page_up_moves_older() {
        assert_eq!(chat_scroll_position(100, 20, 5), 75);
    }

    #[test]
    fn chat_scroll_clamps_when_content_fits() {
        assert_eq!(chat_scroll_position(5, 20, 0), 0);
        assert_eq!(chat_scroll_position(5, 20, 10), 0);
    }

    #[test]
    fn wrap_text_splits_long_unbroken_tokens() {
        assert_eq!(
            wrap_text("/add Sydney abcdefghij", 6),
            vec![
                "/add".to_string(),
                "Sydney".to_string(),
                "abcdef".to_string(),
                "ghij".to_string()
            ]
        );
    }

    #[test]
    fn wrap_text_keeps_words_when_possible() {
        assert_eq!(
            wrap_text("Send this to your contact", 12),
            vec!["Send this to".to_string(), "your contact".to_string()]
        );
    }
}

fn draw_input(f: &mut Frame, area: Rect, app: &App) {
    let input = Paragraph::new(Span::raw(&app.input)).block(
        Block::default()
            .title("Input")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Yellow)),
    );
    f.render_widget(input, area);
}

fn draw_footer(f: &mut Frame, area: Rect, app: &App) {
    let contact_name = if app.contacts.is_empty() || app.contacts[0] == "(no contacts)" {
        "(no contacts)".into()
    } else {
        app.contacts[app.selected_contact].clone()
    };
    let mut spans = vec![
        Span::styled(
            format!(" {} ", app.profile_name),
            Style::default().fg(Color::White).bg(Color::Blue),
        ),
        Span::raw(" | "),
        Span::styled("Tab", Style::default().fg(Color::Cyan)),
        Span::raw(":contact "),
        Span::styled("PgUp/PgDn", Style::default().fg(Color::Cyan)),
        Span::raw(":scroll "),
        Span::styled("/help", Style::default().fg(Color::Cyan)),
        Span::raw(":cmds "),
        Span::raw(" | → "),
        Span::styled(
            contact_name,
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        ),
    ];
    if let Some(ref err) = app.last_error {
        spans.push(Span::raw(" | "));
        spans.push(Span::styled(
            format!("⚠ {}", err),
            Style::default().fg(Color::Red),
        ));
    }
    let text = Line::from(spans);
    let footer = Paragraph::new(text).style(Style::default().bg(Color::DarkGray));
    f.render_widget(footer, area);
}
