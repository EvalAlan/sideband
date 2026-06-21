// FFI-friendly API wrappers — take &str, convert to &Path internally
// Function names are prefixed with api_ to avoid shadowing crate-internal functions
use std::ffi::{CStr, CString};
use std::os::raw::c_char;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::Mutex;

use anyhow::{anyhow, Result};
use rusqlite::params;

use crate::transport::tor::TorTransport;
use crate::types::{ApiContact, ApiGroup, ApiMessage, ApiStatus};

type ListenerStatusCallback = extern "C" fn(status: *const c_char, onion: *const c_char);

struct MobileSendCommand {
    to: String,
    body: String,
    response: tokio::sync::oneshot::Sender<Result<(), String>>,
}

struct ListenerState {
    quit_tx: Option<tokio::sync::oneshot::Sender<()>>,
    send_tx: Option<tokio::sync::mpsc::Sender<MobileSendCommand>>,
    handle: Option<std::thread::JoinHandle<()>>,
}

struct ListenerStatus {
    status: String,
    onion: String,
}

static LISTENER_STATE: Mutex<Option<ListenerState>> = Mutex::new(None);
static LISTENER_STATUS: Mutex<ListenerStatus> = Mutex::new(ListenerStatus {
    status: String::new(),
    onion: String::new(),
});

fn set_listener_status(status: &str, onion: &str) {
    if let Ok(mut guard) = LISTENER_STATUS.lock() {
        guard.status = status.to_string();
        guard.onion = onion.to_string();
    }
}

fn get_listener_status() -> (String, String) {
    if let Ok(guard) = LISTENER_STATUS.lock() {
        return (guard.status.clone(), guard.onion.clone());
    }
    (String::new(), String::new())
}

fn expand_profile(profile_path: &str) -> PathBuf {
    crate::expand_home(Path::new(profile_path))
}

pub fn api_init_profile(profile_path: &str, display_name: &str) -> Result<()> {
    let profile = expand_profile(profile_path);
    if crate::identity_path(&profile).exists() {
        return Ok(());
    }
    let name = display_name.trim();
    let chosen = if name.is_empty() {
        crate::default_display_name(&profile)
    } else {
        name.to_string()
    };
    crate::init_profile_with_name(&profile, &chosen)
}

pub fn api_list_contacts(profile_path: &str) -> Result<Vec<ApiContact>> {
    let profile = expand_profile(profile_path);
    let contacts = crate::load_contacts(&profile)?;
    let mut out: Vec<ApiContact> = contacts
        .values()
        .map(|c| ApiContact {
            name: c.name.clone(),
            onion: c.onion.clone(),
            ed25519_pubkey_b64: c.pubkey_b64.clone(),
            x25519_pubkey_b64: c.x25519_pubkey_b64.clone(),
            pending: c.pending,
            blocked: c.blocked,
        })
        .collect();
    out.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(out)
}

pub fn api_list_groups(profile_path: &str) -> Result<Vec<ApiGroup>> {
    let profile = expand_profile(profile_path);
    let groups = crate::load_groups(&profile)?;
    let mut out: Vec<ApiGroup> = groups
        .into_iter()
        .map(|g| ApiGroup {
            id: g.id,
            title: g.title,
            members: g.members.into_iter().map(|m| m.contact).collect(),
        })
        .collect();
    out.sort_by(|a, b| a.title.cmp(&b.title));
    Ok(out)
}

pub fn api_add_contact(profile_path: &str, contact: ApiContact) -> Result<()> {
    let profile = expand_profile(profile_path);
    let x25519 = contact
        .x25519_pubkey_b64
        .ok_or_else(|| anyhow!("x25519 pubkey is required"))?;
    crate::contact_add(
        &profile,
        &contact.name,
        &contact.onion,
        &contact.ed25519_pubkey_b64,
        &x25519,
    )
}

pub fn api_delete_contact(profile_path: &str, name: &str) -> Result<bool> {
    let profile = expand_profile(profile_path);
    crate::contact_delete(&profile, name)
}

pub fn api_accept_contact(profile_path: &str, name: &str) -> Result<bool> {
    let profile = expand_profile(profile_path);
    crate::contact_accept(&profile, name)
}

pub fn api_block_contact(profile_path: &str, name: &str) -> Result<bool> {
    let profile = expand_profile(profile_path);
    crate::contact_block(&profile, name)
}

pub fn api_unblock_contact(profile_path: &str, name: &str) -> Result<bool> {
    let profile = expand_profile(profile_path);
    crate::contact_unblock(&profile, name)
}

pub async fn api_send_message(profile_path: &str, to: &str, body: &str) -> Result<()> {
    let profile = expand_profile(profile_path);
    let listener_send_tx = {
        let guard = LISTENER_STATE
            .lock()
            .map_err(|_| anyhow!("listener mutex poisoned"))?;
        guard.as_ref().and_then(|state| state.send_tx.clone())
    };

    if let Some(send_tx) = listener_send_tx {
        let (response_tx, response_rx) = tokio::sync::oneshot::channel::<Result<(), String>>();
        send_tx
            .send(MobileSendCommand {
                to: to.to_string(),
                body: body.to_string(),
                response: response_tx,
            })
            .await
            .map_err(|_| anyhow!("mobile listener send channel is closed"))?;
        return response_rx
            .await
            .map_err(|_| anyhow!("mobile listener send response was dropped"))?
            .map_err(|e| anyhow!(e));
    }

    let onion = crate::resolve_to(&profile, to)?;
    let tor_client = TorTransport::bootstrap(&profile).await?;
    crate::send(&profile, &onion, body, to, None, tor_client, false).await
}

pub async fn api_send_file(profile_path: &str, to: &str, file_path: &str) -> Result<()> {
    let profile = expand_profile(profile_path);
    let tor_client = TorTransport::bootstrap(&profile).await?;
    crate::send_file(&profile, to, file_path, None, tor_client).await
}

pub fn api_list_messages(
    profile_path: &str,
    contact: Option<&str>,
    limit: usize,
) -> Result<Vec<ApiMessage>> {
    let profile = expand_profile(profile_path);
    let rows = crate::load_history(&profile, contact, limit)?;
    Ok(rows
        .into_iter()
        .map(|r| ApiMessage {
            id: r.id,
            direction: r.direction,
            contact: r.contact,
            onion: r.onion,
            body: r.body,
            timestamp_ms: r.timestamp_ms,
            status: match r.status {
                2 => "failed".to_string(),
                1 => "delivered".to_string(),
                _ => "sent".to_string(),
            },
            created_at: r.created_at,
            group_id: if r.conversation_kind == "group" {
                r.conversation_id
            } else {
                String::new()
            },
        })
        .collect())
}

pub fn api_list_group_messages(
    profile_path: &str,
    group_id: &str,
    limit: usize,
) -> Result<Vec<ApiMessage>> {
    let profile = expand_profile(profile_path);
    let conn = crate::init_db(&profile)?;
    let mut stmt = conn.prepare(
        "SELECT id, direction, contact, onion, body, timestamp_ms, status, created_at, conversation_kind, conversation_id
         FROM messages
         WHERE conversation_kind = 'group' AND conversation_id = ?1
         ORDER BY timestamp_ms DESC LIMIT ?2",
    )?;
    let rows = stmt.query_map(params![group_id, limit as i64], |r| {
        Ok(crate::HistoryRow {
            id: r.get(0)?,
            direction: r.get(1)?,
            contact: r.get(2)?,
            onion: r.get(3)?,
            body: r.get(4)?,
            timestamp_ms: r.get(5)?,
            status: r.get(6)?,
            created_at: r.get(7)?,
            conversation_kind: r.get(8)?,
            conversation_id: r.get(9)?,
        })
    })?;
    let mut out = Vec::new();
    for row in rows {
        let r = row?;
        out.push(ApiMessage {
            id: r.id,
            direction: r.direction,
            contact: r.contact,
            onion: r.onion,
            body: r.body,
            timestamp_ms: r.timestamp_ms,
            status: match r.status {
                2 => "failed".to_string(),
                1 => "delivered".to_string(),
                _ => "sent".to_string(),
            },
            created_at: r.created_at,
            group_id: if r.conversation_kind == "group" {
                r.conversation_id
            } else {
                String::new()
            },
        });
    }
    out.reverse();
    Ok(out)
}

pub fn api_list_transfers(profile_path: &str) -> Result<Vec<String>> {
    let profile = expand_profile(profile_path);
    crate::list_transfers(&profile)
}

pub async fn api_resume_transfer(profile_path: &str, hash: &str) -> Result<bool> {
    let profile = expand_profile(profile_path);
    let Some((contact, file_path)) = crate::outbound_transfer_target(&profile, hash)? else {
        return Ok(false);
    };
    let tor_client = TorTransport::bootstrap(&profile).await?;
    crate::send_file(
        &profile,
        &contact,
        &file_path,
        None,
        Arc::clone(&tor_client),
    )
    .await?;
    Ok(true)
}

pub fn api_cancel_transfer(profile_path: &str, hash: &str) -> Result<bool> {
    let profile = expand_profile(profile_path);
    crate::cancel_outbound_transfer(&profile, hash)
}

pub fn api_status(profile_path: &str) -> Result<ApiStatus> {
    let profile = expand_profile(profile_path);
    let contacts = crate::load_contacts(&profile)?;
    let transfers = crate::list_transfers(&profile)?;
    let (listener_status, listener_onion) = get_listener_status();
    Ok(ApiStatus {
        profile: profile.display().to_string(),
        display_name: crate::load_display_name(&profile)?,
        contact_count: contacts.len(),
        transfer_count: transfers.len(),
        listener_status,
        listener_onion,
        listener_running: LISTENER_STATE
            .lock()
            .map(|guard| guard.is_some())
            .unwrap_or(false),
    })
}

fn cstr_arg<'a>(ptr: *const c_char, name: &str) -> Result<&'a str> {
    if ptr.is_null() {
        return Err(anyhow!("{name} is null"));
    }
    unsafe { CStr::from_ptr(ptr) }
        .to_str()
        .map_err(|e| anyhow!("{name} is not valid UTF-8: {e}"))
}

fn json_response<T: serde::Serialize>(result: Result<T>) -> *mut c_char {
    let value = match result {
        Ok(data) => serde_json::json!({ "ok": true, "data": data }),
        Err(err) => serde_json::json!({ "ok": false, "error": err.to_string() }),
    };
    let text = value.to_string().replace('\0', "");
    CString::new(text)
        .expect("JSON response contains no NUL bytes")
        .into_raw()
}

#[no_mangle]
pub extern "C" fn sideband_api_free_string(ptr: *mut c_char) {
    if ptr.is_null() {
        return;
    }
    unsafe {
        let _ = CString::from_raw(ptr);
    }
}

#[no_mangle]
pub extern "C" fn sideband_api_init_profile(
    profile_path: *const c_char,
    display_name: *const c_char,
) -> *mut c_char {
    json_response((|| {
        api_init_profile(
            cstr_arg(profile_path, "profile_path")?,
            cstr_arg(display_name, "display_name")?,
        )
    })())
}

#[no_mangle]
pub extern "C" fn sideband_api_status(profile_path: *const c_char) -> *mut c_char {
    json_response((|| api_status(cstr_arg(profile_path, "profile_path")?))())
}

#[no_mangle]
pub extern "C" fn sideband_api_list_contacts(profile_path: *const c_char) -> *mut c_char {
    json_response((|| {
        api_list_contacts(cstr_arg(profile_path, "profile_path")?)
    })())
}

#[no_mangle]
pub extern "C" fn sideband_api_list_groups(profile_path: *const c_char) -> *mut c_char {
    json_response((|| {
        api_list_groups(cstr_arg(profile_path, "profile_path")?)
    })())
}

#[no_mangle]
pub extern "C" fn sideband_api_list_group_messages(
    profile_path: *const c_char,
    group_id: *const c_char,
    limit: usize,
) -> *mut c_char {
    json_response((|| {
        api_list_group_messages(
            cstr_arg(profile_path, "profile_path")?,
            cstr_arg(group_id, "group_id")?,
            limit,
        )
    })())
}

#[no_mangle]
pub extern "C" fn sideband_api_list_messages(
    profile_path: *const c_char,
    contact: *const c_char,
    limit: usize,
) -> *mut c_char {
    json_response((|| {
        let contact = cstr_arg(contact, "contact")?;
        api_list_messages(
            cstr_arg(profile_path, "profile_path")?,
            if contact.trim().is_empty() {
                None
            } else {
                Some(contact)
            },
            limit,
        )
    })())
}

#[no_mangle]
pub extern "C" fn sideband_api_send_message(
    profile_path: *const c_char,
    to: *const c_char,
    body: *const c_char,
) -> *mut c_char {
    json_response((|| {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()?;
        runtime.block_on(api_send_message(
            cstr_arg(profile_path, "profile_path")?,
            cstr_arg(to, "to")?,
            cstr_arg(body, "body")?,
        ))
    })())
}

#[no_mangle]
pub extern "C" fn sideband_api_add_contact(
    profile_path: *const c_char,
    name: *const c_char,
    onion: *const c_char,
    ed25519_pubkey_b64: *const c_char,
    x25519_pubkey_b64: *const c_char,
) -> *mut c_char {
    json_response((|| {
        api_add_contact(
            cstr_arg(profile_path, "profile_path")?,
            ApiContact {
                name: cstr_arg(name, "name")?.to_string(),
                onion: cstr_arg(onion, "onion")?.to_string(),
                ed25519_pubkey_b64: cstr_arg(ed25519_pubkey_b64, "ed25519_pubkey_b64")?
                    .to_string(),
                x25519_pubkey_b64: Some(
                    cstr_arg(x25519_pubkey_b64, "x25519_pubkey_b64")?.to_string(),
                ),
                pending: false,
                blocked: false,
            },
        )
    })())
}

#[no_mangle]
pub extern "C" fn sideband_api_delete_contact(
    profile_path: *const c_char,
    name: *const c_char,
) -> *mut c_char {
    json_response((|| {
        api_delete_contact(
            cstr_arg(profile_path, "profile_path")?,
            cstr_arg(name, "name")?,
        )
    })())
}

#[no_mangle]
pub extern "C" fn sideband_api_accept_contact(
    profile_path: *const c_char,
    name: *const c_char,
) -> *mut c_char {
    json_response((|| {
        api_accept_contact(
            cstr_arg(profile_path, "profile_path")?,
            cstr_arg(name, "name")?,
        )
    })())
}

#[no_mangle]
pub extern "C" fn sideband_api_block_contact(
    profile_path: *const c_char,
    name: *const c_char,
) -> *mut c_char {
    json_response((|| {
        api_block_contact(
            cstr_arg(profile_path, "profile_path")?,
            cstr_arg(name, "name")?,
        )
    })())
}

#[no_mangle]
pub extern "C" fn sideband_api_unblock_contact(
    profile_path: *const c_char,
    name: *const c_char,
) -> *mut c_char {
    json_response((|| {
        api_unblock_contact(
            cstr_arg(profile_path, "profile_path")?,
            cstr_arg(name, "name")?,
        )
    })())
}

#[no_mangle]
pub extern "C" fn sideband_api_create_group(
    profile_path: *const c_char,
    title: *const c_char,
    members_json: *const c_char,
) -> *mut c_char {
    json_response((|| {
        let profile = cstr_arg(profile_path, "profile_path")?;
        let title = cstr_arg(title, "title")?;
        let members_raw = cstr_arg(members_json, "members_json")?;
        let members: Vec<String> = serde_json::from_str(members_raw)
            .map_err(|e| anyhow!("parse members JSON: {e}"))?;
        let group = crate::create_group(Path::new(profile), title, &members)?;
        Ok(group)
    })())
}

#[no_mangle]
pub extern "C" fn sideband_api_delete_group(
    profile_path: *const c_char,
    group_id: *const c_char,
) -> *mut c_char {
    json_response((|| {
        let profile = cstr_arg(profile_path, "profile_path")?;
        let group_id = cstr_arg(group_id, "group_id")?;
        let group = crate::delete_group(Path::new(profile), group_id)?;
        Ok(group)
    })())
}

#[no_mangle]
pub extern "C" fn sideband_api_clear_history(
    profile_path: *const c_char,
    contact: *const c_char,
) -> *mut c_char {
    json_response((|| {
        let profile = cstr_arg(profile_path, "profile_path")?;
        let contact = if contact.is_null() {
            None
        } else {
            Some(cstr_arg(contact, "contact")?)
        };
        crate::clear_history(Path::new(profile), contact)?;
        Ok(())
    })())
}

#[no_mangle]
pub extern "C" fn sideband_api_share_command(
    profile_path: *const c_char,
    onion: *const c_char,
) -> *mut c_char {
    json_response((|| {
        let profile = expand_profile(cstr_arg(profile_path, "profile_path")?);
        let onion = cstr_arg(onion, "onion")?;
        let command = crate::share_command(&profile, onion)?;
        let qr = crate::qr_matrix(&command)?;
        Ok(crate::ShareInfo { command, qr })
    })())
}

#[no_mangle]
pub extern "C" fn sideband_api_listener_start(
    profile_path: *const c_char,
) -> *mut c_char {
    json_response((|| {
        let profile = cstr_arg(profile_path, "profile_path")?;
        let profile_buf = PathBuf::from(profile);

        let mut guard = LISTENER_STATE.lock().map_err(|_| anyhow!("listener mutex poisoned"))?;
        if guard.is_some() {
            return Err(anyhow!("listener already running"));
        }

        let (quit_tx, quit_rx) = tokio::sync::oneshot::channel::<()>();
        let (send_tx, mut send_rx) = tokio::sync::mpsc::channel::<MobileSendCommand>(64);
        let profile_for_task = profile_buf.clone();
        set_listener_status("listener starting", "");
        let handle = std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .expect("failed to build tokio runtime for listener");
            runtime.block_on(async move {
                let tor_client = match TorTransport::bootstrap(&profile_for_task).await {
                    Ok(c) => c,
                    Err(e) => {
                        set_listener_status(&format!("listener failed: {e}"), "");
                        return;
                    }
                };
                set_listener_status("tor bootstrapping", "");
                let send_profile = profile_for_task.clone();
                let send_client = tor_client.clone();
                let send_lock = Arc::new(tokio::sync::Mutex::new(()));
                let send_task = tokio::spawn(async move {
                    while let Some(cmd) = send_rx.recv().await {
                        let result = match crate::resolve_to(&send_profile, &cmd.to) {
                            Ok(onion) => {
                                let _guard = send_lock.lock().await;
                                crate::send(
                                    &send_profile,
                                    &onion,
                                    &cmd.body,
                                    &cmd.to,
                                    None,
                                    send_client.clone(),
                                    false,
                                )
                                .await
                                .map_err(|e| e.to_string())
                            }
                            Err(e) => Err(e.to_string()),
                        };
                        let _ = cmd.response.send(result);
                    }
                });
                let (tui_tx, mut tui_rx) = tokio::sync::mpsc::channel::<crate::TuiEvent>(256);
                let status_task = tokio::spawn(async move {
                    while let Some(event) = tui_rx.recv().await {
                        match event {
                            crate::TuiEvent::StatusUpdate(text) => {
                                if let Some(onion) = text.strip_prefix("onion=") {
                                    set_listener_status("listening", onion);
                                } else {
                                    let (_, onion) = get_listener_status();
                                    set_listener_status(&text, &onion);
                                }
                            }
                            crate::TuiEvent::InboundMessage { .. }
                            | crate::TuiEvent::InboundGroupMessage { .. } => {
                                let (_, onion) = get_listener_status();
                                set_listener_status("message received", &onion);
                            }
                            crate::TuiEvent::OutboundMessage { .. } => {}
                        }
                    }
                });

                match crate::serve(&profile_for_task, tui_tx, quit_rx, tor_client, false).await {
                    Ok(()) => set_listener_status("listener stopped", ""),
                    Err(e) => set_listener_status(&format!("listener failed: {e}"), ""),
                }
                send_task.abort();
                status_task.abort();
            });
        });

        *guard = Some(ListenerState {
            quit_tx: Some(quit_tx),
            send_tx: Some(send_tx),
            handle: Some(handle),
        });
        Ok(())
    })())
}

#[no_mangle]
pub extern "C" fn sideband_api_listener_stop() -> *mut c_char {
    json_response((|| {
        let mut guard = LISTENER_STATE.lock().map_err(|_| anyhow!("listener mutex poisoned"))?;
        let mut state = guard.take().ok_or_else(|| anyhow!("listener not running"))?;
        let quit_tx = state.quit_tx.take();
        if let Some(tx) = quit_tx {
            let _ = tx.send(());
        }
        set_listener_status("listener stopped", "");
        drop(state);
        Ok(())
    })())
}

