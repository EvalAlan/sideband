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
    payload: MobileSendPayload,
    response: tokio::sync::oneshot::Sender<Result<(), String>>,
}

enum MobileSendPayload {
    Message(String),
    File(String),
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

/// Shared multi-thread runtime used to spawn fire-and-forget network work when
/// no listener is running. This keeps FFI entry points from ever blocking on
/// Tor I/O on the caller's (Flutter UI isolate) thread.
static SHARED_RUNTIME: std::sync::OnceLock<tokio::runtime::Runtime> = std::sync::OnceLock::new();

fn shared_runtime() -> Result<&'static tokio::runtime::Runtime> {
    if let Some(rt) = SHARED_RUNTIME.get() {
        return Ok(rt);
    }
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|e| anyhow!("failed to build shared runtime: {e}"))?;
    // If two threads race, keep whichever landed first; both are equivalent.
    let _ = SHARED_RUNTIME.set(rt);
    SHARED_RUNTIME
        .get()
        .ok_or_else(|| anyhow!("shared runtime unavailable"))
}

static LISTENER_STATE: Mutex<Option<ListenerState>> = Mutex::new(None);
static LISTENER_STATUS: Mutex<ListenerStatus> = Mutex::new(ListenerStatus {
    status: String::new(),
    onion: String::new(),
});

fn listener_is_running(state: &ListenerState) -> bool {
    state
        .handle
        .as_ref()
        .map(|handle| !handle.is_finished())
        .unwrap_or(false)
}

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
            ratchet_active: crate::ratchet_is_active(&profile, &c.name),
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
                payload: MobileSendPayload::Message(body.to_string()),
                response: response_tx,
            })
            .await
            .map_err(|_| anyhow!("mobile listener send channel is closed"))?;
        // Enqueue-and-return: waiting for the full Tor round-trip here would
        // freeze Flutter's UI isolate (ANR). Delivery status flows back to the
        // app via listener status events, and the listener serializes the send.
        drop(response_rx);
        return Ok(());
    }

    // No listener: never bootstrap Tor on the caller's thread. Persist to the
    // retry queue so the message is delivered when the listener next runs, and
    // return immediately.
    let onion = crate::resolve_to(&profile, to)?;
    crate::enqueue_retry(&profile, to, &onion, body, "queued: no listener running")?;
    Ok(())
}

pub async fn api_send_file(profile_path: &str, to: &str, file_path: &str) -> Result<()> {
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
                payload: MobileSendPayload::File(file_path.to_string()),
                response: response_tx,
            })
            .await
            .map_err(|_| anyhow!("mobile listener send channel is closed"))?;
        // File transfers can take many seconds because chunked sends wait for
        // ACKs. The Android FFI call runs on Flutter's UI isolate, so waiting
        // here freezes the app hard enough for Android to show "not responding".
        // Queue the work onto the listener-owned runtime and return once it is
        // accepted; the listener still serializes the actual transfer and logs
        // any delivery failure.
        drop(response_rx);
        return Ok(());
    }

    // No listener: bootstrapping Tor here would run on the caller's UI isolate
    // and hang the app. Spawn the bootstrap + transfer onto the shared runtime
    // and return immediately; the transfer persists resumable state as it goes.
    let rt = shared_runtime()?;
    let profile_owned = profile.clone();
    let to_owned = to.to_string();
    let file_owned = file_path.to_string();
    rt.spawn(async move {
        match TorTransport::bootstrap(&profile_owned).await {
            Ok(tor_client) => {
                if let Err(e) =
                    crate::send_file(&profile_owned, &to_owned, &file_owned, None, tor_client).await
                {
                    tracing::error!(error=%e, to=%to_owned, "background file send failed");
                }
            }
            Err(e) => tracing::error!(error=%e, "background file send: tor bootstrap failed"),
        }
    });
    Ok(())
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

pub fn api_cancel_transfer(profile_path: &str, hash: &str) -> Result<bool> {
    let profile = expand_profile(profile_path);
    crate::cancel_outbound_transfer(&profile, hash)
}

/// Number of messages currently queued for background retry.
pub fn api_retry_status(profile_path: &str) -> Result<usize> {
    let profile = expand_profile(profile_path);
    crate::retry_queue_len(&profile)
}

/// Enqueue a group message fan-out and return immediately. Never blocks on Tor
/// I/O on the caller's thread (mirrors the file-send non-blocking pattern).
pub fn api_send_group_message(profile_path: &str, group_id: &str, message: &str) -> Result<()> {
    let profile = expand_profile(profile_path);
    // Validate the group exists before spawning so obvious errors surface synchronously.
    let _ = crate::resolve_group(&profile, group_id)?;
    let rt = shared_runtime()?;
    let group = group_id.to_string();
    let body = message.to_string();
    rt.spawn(async move {
        match TorTransport::bootstrap(&profile).await {
            Ok(tor_client) => {
                if let Err(e) = crate::send_group(&profile, &group, &body, tor_client, false).await
                {
                    tracing::error!(error=%e, group=%group, "background group send failed");
                }
            }
            Err(e) => tracing::error!(error=%e, "background group send: tor bootstrap failed"),
        }
    });
    Ok(())
}

/// Enqueue a group file fan-out and return immediately.
pub fn api_send_group_file(profile_path: &str, group_id: &str, path: &str) -> Result<()> {
    let profile = expand_profile(profile_path);
    let _ = crate::resolve_group(&profile, group_id)?;
    let rt = shared_runtime()?;
    let group = group_id.to_string();
    let file = path.to_string();
    rt.spawn(async move {
        match TorTransport::bootstrap(&profile).await {
            Ok(tor_client) => {
                if let Err(e) = crate::send_file_to_group(&profile, &group, &file, tor_client).await
                {
                    tracing::error!(error=%e, group=%group, "background group file send failed");
                }
            }
            Err(e) => tracing::error!(error=%e, "background group file send: tor bootstrap failed"),
        }
    });
    Ok(())
}

pub fn api_rename_group(
    profile_path: &str,
    group_id: &str,
    title: &str,
) -> Result<crate::GroupInfo> {
    let profile = expand_profile(profile_path);
    crate::rename_group(&profile, group_id, title)
}

pub fn api_group_add_member(
    profile_path: &str,
    group_id: &str,
    member: &str,
) -> Result<crate::GroupInfo> {
    let profile = expand_profile(profile_path);
    crate::add_group_member(&profile, group_id, member)
}

pub fn api_group_remove_member(
    profile_path: &str,
    group_id: &str,
    member: &str,
) -> Result<crate::GroupInfo> {
    let profile = expand_profile(profile_path);
    crate::remove_group_member(&profile, group_id, member)
}

/// Leave a group. The member-notification fan-out (which needs Tor) is spawned
/// onto the shared runtime so this never blocks the caller; the local group
/// snapshot is resolved and returned synchronously.
pub fn api_leave_group(profile_path: &str, group_id: &str) -> Result<crate::GroupInfo> {
    let profile = expand_profile(profile_path);
    let group = crate::resolve_group(&profile, group_id)?;
    let rt = shared_runtime()?;
    let group_ref = group_id.to_string();
    let profile_owned = profile.clone();
    rt.spawn(async move {
        match TorTransport::bootstrap(&profile_owned).await {
            Ok(tor_client) => {
                if let Err(e) = crate::leave_group(&profile_owned, &group_ref, tor_client).await {
                    tracing::error!(error=%e, "background group leave failed");
                }
            }
            Err(e) => tracing::error!(error=%e, "background group leave: tor bootstrap failed"),
        }
    });
    Ok(group)
}

/// Resume an outbound transfer in the background and return immediately. Returns
/// `false` only when there is no persisted transfer for `hash`.
pub fn api_resume_transfer_bg(profile_path: &str, hash: &str) -> Result<bool> {
    let profile = expand_profile(profile_path);
    let Some((contact, file_path)) = crate::outbound_transfer_target(&profile, hash)? else {
        return Ok(false);
    };
    let rt = shared_runtime()?;
    rt.spawn(async move {
        match TorTransport::bootstrap(&profile).await {
            Ok(tor_client) => {
                if let Err(e) =
                    crate::send_file(&profile, &contact, &file_path, None, tor_client).await
                {
                    tracing::error!(error=%e, "background resume transfer failed");
                }
            }
            Err(e) => tracing::error!(error=%e, "background resume: tor bootstrap failed"),
        }
    });
    Ok(true)
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
            .map(|guard| guard.as_ref().map(listener_is_running).unwrap_or(false))
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

/// Free a string previously returned by any `sideband_api_*` function.
///
/// # Safety
/// `ptr` must be null or a pointer returned by a `sideband_api_*` call that has
/// not already been freed. Passing any other pointer is undefined behaviour.
#[no_mangle]
pub unsafe extern "C" fn sideband_api_free_string(ptr: *mut c_char) {
    if ptr.is_null() {
        return;
    }
    let _ = CString::from_raw(ptr);
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
        // These async fns enqueue-and-return without awaiting network I/O, so
        // block_on completes immediately and never freezes the caller.
        shared_runtime()?.block_on(api_send_message(
            cstr_arg(profile_path, "profile_path")?,
            cstr_arg(to, "to")?,
            cstr_arg(body, "body")?,
        ))
    })())
}

#[no_mangle]
pub extern "C" fn sideband_api_send_file(
    profile_path: *const c_char,
    to: *const c_char,
    file_path: *const c_char,
) -> *mut c_char {
    json_response((|| {
        shared_runtime()?.block_on(api_send_file(
            cstr_arg(profile_path, "profile_path")?,
            cstr_arg(to, "to")?,
            cstr_arg(file_path, "file_path")?,
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
                ed25519_pubkey_b64: cstr_arg(ed25519_pubkey_b64, "ed25519_pubkey_b64")?.to_string(),
                x25519_pubkey_b64: Some(
                    cstr_arg(x25519_pubkey_b64, "x25519_pubkey_b64")?.to_string(),
                ),
                pending: false,
                blocked: false,
                ratchet_active: false,
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
        let members: Vec<String> =
            serde_json::from_str(members_raw).map_err(|e| anyhow!("parse members JSON: {e}"))?;
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
pub extern "C" fn sideband_api_listener_start(profile_path: *const c_char) -> *mut c_char {
    json_response((|| {
        let profile = cstr_arg(profile_path, "profile_path")?;
        let profile_buf = PathBuf::from(profile);

        let mut guard = LISTENER_STATE
            .lock()
            .map_err(|_| anyhow!("listener mutex poisoned"))?;
        if guard.as_ref().map(listener_is_running).unwrap_or(false) {
            return Err(anyhow!("listener already running"));
        }
        if guard.is_some() {
            *guard = None;
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
                        let _guard = send_lock.lock().await;
                        let result = match cmd.payload {
                            MobileSendPayload::Message(body) => {
                                match crate::resolve_to(&send_profile, &cmd.to) {
                                    Ok(onion) => crate::send(
                                        &send_profile,
                                        &onion,
                                        &body,
                                        &cmd.to,
                                        None,
                                        send_client.clone(),
                                        false,
                                    )
                                    .await
                                    .map_err(|e| e.to_string()),
                                    Err(e) => Err(e.to_string()),
                                }
                            }
                            MobileSendPayload::File(file_path) => crate::send_file(
                                &send_profile,
                                &cmd.to,
                                &file_path,
                                None,
                                send_client.clone(),
                            )
                            .await
                            .map_err(|e| e.to_string()),
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

                match crate::serve(&profile_for_task, tui_tx, quit_rx, tor_client, false, None).await {
                    Ok(()) => set_listener_status("listener stopped", ""),
                    Err(e) => set_listener_status(&format!("listener failed: {e}"), ""),
                }
                send_task.abort();
                status_task.abort();
            });
            if let Ok(mut guard) = LISTENER_STATE.lock() {
                *guard = None;
            }
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
        let mut guard = LISTENER_STATE
            .lock()
            .map_err(|_| anyhow!("listener mutex poisoned"))?;
        let mut state = guard
            .take()
            .ok_or_else(|| anyhow!("listener not running"))?;
        let quit_tx = state.quit_tx.take();
        if let Some(tx) = quit_tx {
            let _ = tx.send(());
        }
        set_listener_status("listener stopped", "");
        drop(state);
        Ok(())
    })())
}

#[no_mangle]
pub extern "C" fn sideband_api_retry_status(profile_path: *const c_char) -> *mut c_char {
    json_response((|| {
        let queued = api_retry_status(cstr_arg(profile_path, "profile_path")?)?;
        Ok(serde_json::json!({ "queued": queued }))
    })())
}

#[no_mangle]
pub extern "C" fn sideband_api_send_group_message(
    profile_path: *const c_char,
    group_id: *const c_char,
    message: *const c_char,
) -> *mut c_char {
    json_response((|| {
        api_send_group_message(
            cstr_arg(profile_path, "profile_path")?,
            cstr_arg(group_id, "group_id")?,
            cstr_arg(message, "message")?,
        )
    })())
}

#[no_mangle]
pub extern "C" fn sideband_api_send_group_file(
    profile_path: *const c_char,
    group_id: *const c_char,
    path: *const c_char,
) -> *mut c_char {
    json_response((|| {
        api_send_group_file(
            cstr_arg(profile_path, "profile_path")?,
            cstr_arg(group_id, "group_id")?,
            cstr_arg(path, "path")?,
        )
    })())
}

#[no_mangle]
pub extern "C" fn sideband_api_rename_group(
    profile_path: *const c_char,
    group_id: *const c_char,
    title: *const c_char,
) -> *mut c_char {
    json_response((|| {
        api_rename_group(
            cstr_arg(profile_path, "profile_path")?,
            cstr_arg(group_id, "group_id")?,
            cstr_arg(title, "title")?,
        )
    })())
}

#[no_mangle]
pub extern "C" fn sideband_api_group_add_member(
    profile_path: *const c_char,
    group_id: *const c_char,
    member: *const c_char,
) -> *mut c_char {
    json_response((|| {
        api_group_add_member(
            cstr_arg(profile_path, "profile_path")?,
            cstr_arg(group_id, "group_id")?,
            cstr_arg(member, "member")?,
        )
    })())
}

#[no_mangle]
pub extern "C" fn sideband_api_group_remove_member(
    profile_path: *const c_char,
    group_id: *const c_char,
    member: *const c_char,
) -> *mut c_char {
    json_response((|| {
        api_group_remove_member(
            cstr_arg(profile_path, "profile_path")?,
            cstr_arg(group_id, "group_id")?,
            cstr_arg(member, "member")?,
        )
    })())
}

#[no_mangle]
pub extern "C" fn sideband_api_leave_group(
    profile_path: *const c_char,
    group_id: *const c_char,
) -> *mut c_char {
    json_response((|| {
        api_leave_group(
            cstr_arg(profile_path, "profile_path")?,
            cstr_arg(group_id, "group_id")?,
        )
    })())
}

#[no_mangle]
pub extern "C" fn sideband_api_list_transfers(profile_path: *const c_char) -> *mut c_char {
    json_response((|| {
        api_list_transfers(cstr_arg(profile_path, "profile_path")?)
    })())
}

#[no_mangle]
pub extern "C" fn sideband_api_resume_transfer(
    profile_path: *const c_char,
    hash: *const c_char,
) -> *mut c_char {
    json_response((|| {
        api_resume_transfer_bg(
            cstr_arg(profile_path, "profile_path")?,
            cstr_arg(hash, "hash")?,
        )
    })())
}

#[no_mangle]
pub extern "C" fn sideband_api_cancel_transfer(
    profile_path: *const c_char,
    hash: *const c_char,
) -> *mut c_char {
    json_response((|| {
        api_cancel_transfer(
            cstr_arg(profile_path, "profile_path")?,
            cstr_arg(hash, "hash")?,
        )
    })())
}

#[cfg(test)]
mod tests {
    use super::*;

    // Async-aware lock so it can be held across .await without the
    // clippy::await_holding_lock hazard a std Mutex would introduce.
    static API_TEST_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

    #[tokio::test(flavor = "current_thread")]
    async fn api_send_file_with_listener_enqueues_without_waiting_for_delivery() {
        let _test_guard = API_TEST_LOCK.lock().await;
        let (send_tx, mut send_rx) = tokio::sync::mpsc::channel::<MobileSendCommand>(1);
        {
            let mut guard = LISTENER_STATE.lock().unwrap();
            *guard = Some(ListenerState {
                quit_tx: None,
                send_tx: Some(send_tx),
                handle: None,
            });
        }

        let result = tokio::time::timeout(
            std::time::Duration::from_millis(100),
            api_send_file("/tmp/sideband-test-profile", "bob", "/tmp/photo.jpg"),
        )
        .await;
        assert!(result.is_ok(), "api_send_file waited for delivery response");
        result.unwrap().unwrap();

        let cmd = send_rx
            .try_recv()
            .expect("file send command was not queued");
        assert_eq!(cmd.to, "bob");
        match cmd.payload {
            MobileSendPayload::File(path) => assert_eq!(path, "/tmp/photo.jpg"),
            MobileSendPayload::Message(_) => panic!("queued message payload instead of file"),
        }

        let mut guard = LISTENER_STATE.lock().unwrap();
        *guard = None;
    }

    #[tokio::test(flavor = "current_thread")]
    async fn api_send_message_with_listener_enqueues_without_waiting() {
        let _test_guard = API_TEST_LOCK.lock().await;
        let (send_tx, mut send_rx) = tokio::sync::mpsc::channel::<MobileSendCommand>(1);
        {
            let mut guard = LISTENER_STATE.lock().unwrap();
            *guard = Some(ListenerState {
                quit_tx: None,
                send_tx: Some(send_tx),
                handle: None,
            });
        }

        let result = tokio::time::timeout(
            std::time::Duration::from_millis(100),
            api_send_message("/tmp/sideband-test-profile", "bob", "hi there"),
        )
        .await;
        assert!(result.is_ok(), "api_send_message waited for delivery");
        result.unwrap().unwrap();

        let cmd = send_rx.try_recv().expect("message was not queued");
        assert_eq!(cmd.to, "bob");
        match cmd.payload {
            MobileSendPayload::Message(body) => assert_eq!(body, "hi there"),
            MobileSendPayload::File(_) => panic!("queued file payload instead of message"),
        }

        let mut guard = LISTENER_STATE.lock().unwrap();
        *guard = None;
    }

    #[test]
    fn api_retry_status_reports_queued_count() {
        let dir = tempfile::tempdir().unwrap();
        let profile = dir.path().to_str().unwrap();
        assert_eq!(api_retry_status(profile).unwrap(), 0);
        crate::enqueue_retry(dir.path(), "alice", "alice.onion", "hi", "err").unwrap();
        assert_eq!(api_retry_status(profile).unwrap(), 1);
    }

    #[test]
    fn api_send_group_message_rejects_unknown_group() {
        let dir = tempfile::tempdir().unwrap();
        crate::init_profile(dir.path()).unwrap();
        let profile = dir.path().to_str().unwrap();
        // Unknown group must fail synchronously (before any network spawn).
        assert!(api_send_group_message(profile, "no-such-group", "hi").is_err());
    }
}
